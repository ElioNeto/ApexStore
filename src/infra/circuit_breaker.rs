//! Circuit breaker pattern for ApexStore resilience.
//!
//! Tracks failure/success counts and transitions between three states:
//! - **Closed** — normal operation, calls pass through.
//! - **Open** — failures above threshold; calls are rejected immediately.
//! - **HalfOpen** — after cooldown, a probe call is allowed; outcome decides
//!   whether to close or re-open.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit breaker state machine.
pub struct CircuitBreaker {
    inner: Mutex<Inner>,
}

struct Inner {
    /// Current state.
    state: State,
    /// Consecutive failures in the current window.
    failure_count: u64,
    /// Consecutive successes in the current window (HalfOpen recovery).
    success_count: u64,
    /// Failure threshold to trip from Closed → Open.
    failure_threshold: u64,
    /// Success threshold to recover from HalfOpen → Closed.
    success_threshold: u64,
    /// Cooldown before transitioning from Open → HalfOpen.
    cooldown: Duration,
    /// When the last failure transitioned us to Open.
    opened_at: Option<Instant>,
}

/// Circuit breaker state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given thresholds.
    ///
    /// * `failure_threshold` — consecutive failures before opening.
    /// * `success_threshold` — consecutive successes in HalfOpen before closing.
    /// * `cooldown` — time to wait before transitioning Open → HalfOpen.
    pub fn new(failure_threshold: u64, success_threshold: u64, cooldown: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: State::Closed,
                failure_count: 0,
                success_count: 0,
                failure_threshold,
                success_threshold,
                cooldown,
                opened_at: None,
            }),
        }
    }

    /// Create a circuit breaker with sensible defaults:
    /// - 5 failures to open
    /// - 3 successes to close
    /// - 30 second cooldown
    pub fn default() -> Self {
        Self::new(5, 3, Duration::from_secs(30))
    }

    /// Attempt to execute the closure `f` through the circuit breaker.
    ///
    /// Returns `Ok(T)` on success, or an error string if the circuit is open
    /// or the closure failed.
    pub fn call<T, E, F>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce() -> std::result::Result<T, E>,
        E: std::fmt::Display,
    {
        // Check state before acquiring the lock for read-heavy path.
        let current_state = self.state();
        match current_state {
            State::Open => {
                // Check if cooldown has elapsed → transition to HalfOpen.
                let mut inner = self.inner.lock().unwrap();
                if let Some(opened_at) = inner.opened_at {
                    if opened_at.elapsed() >= inner.cooldown {
                        inner.state = State::HalfOpen;
                        inner.success_count = 0;
                    } else {
                        return Err("circuit breaker is open".to_string());
                    }
                } else {
                    return Err("circuit breaker is open".to_string());
                }
            }
            State::HalfOpen => {
                // Only one probe call is allowed; we let it through.
            }
            State::Closed => { /* pass through */ }
        }

        // Execute the operation.
        match f() {
            Ok(result) => {
                self.record_success();
                Ok(result)
            }
            Err(e) => {
                self.record_failure();
                Err(format!("operation failed: {}", e))
            }
        }
    }

    /// Record a successful call.
    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            State::Closed => {
                // Reset failure counter on success.
                inner.failure_count = 0;
            }
            State::HalfOpen => {
                inner.success_count += 1;
                if inner.success_count >= inner.success_threshold {
                    inner.state = State::Closed;
                    inner.failure_count = 0;
                    inner.success_count = 0;
                    inner.opened_at = None;
                }
            }
            State::Open => {
                // Shouldn't happen, but reset just in case.
                inner.state = State::Closed;
                inner.failure_count = 0;
                inner.success_count = 0;
                inner.opened_at = None;
            }
        }
    }

    /// Record a failed call.
    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            State::Closed => {
                inner.failure_count += 1;
                if inner.failure_count >= inner.failure_threshold {
                    inner.state = State::Open;
                    inner.opened_at = Some(Instant::now());
                }
            }
            State::HalfOpen => {
                // Failure in HalfOpen immediately re-opens.
                inner.state = State::Open;
                inner.opened_at = Some(Instant::now());
                inner.success_count = 0;
            }
            State::Open => {
                // Extend the cooldown window.
                inner.opened_at = Some(Instant::now());
            }
        }
    }

    /// Returns the current state.
    pub fn state(&self) -> State {
        let inner = self.inner.lock().unwrap();
        inner.state
    }

    /// Returns the current failure count.
    pub fn failure_count(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.failure_count
    }

    /// Returns the current success count (used in HalfOpen).
    pub fn success_count(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.success_count
    }

    /// Reset the circuit breaker to Closed state.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = State::Closed;
        inner.failure_count = 0;
        inner.success_count = 0;
        inner.opened_at = None;
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_closed_by_default() {
        let cb = CircuitBreaker::default();
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = CircuitBreaker::new(2, 1, Duration::from_secs(60));
        assert_eq!(cb.state(), State::Closed);

        let result: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert!(result.is_err());
        assert_eq!(cb.failure_count(), 1);
        assert_eq!(cb.state(), State::Closed);

        let result: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert!(result.is_err());
        assert_eq!(cb.failure_count(), 2);
        assert_eq!(cb.state(), State::Open);
    }

    #[test]
    fn test_rejects_when_open() {
        let cb = CircuitBreaker::new(1, 1, Duration::from_secs(60));
        let _: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert_eq!(cb.state(), State::Open);

        let result: Result<(), String> = cb.call(|| Ok::<(), &str>(()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("circuit breaker is open"));
    }

    #[test]
    fn test_half_open_transition() {
        let cb = CircuitBreaker::new(1, 1, Duration::from_millis(10));
        let _: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert_eq!(cb.state(), State::Open);

        // Wait for cooldown
        std::thread::sleep(Duration::from_millis(20));

        // Now the call should be allowed (HalfOpen probe)
        let result: Result<(), String> = cb.call(|| Ok::<(), &str>(()));
        assert!(result.is_ok());
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn test_success_resets_failure_count() {
        let cb = CircuitBreaker::new(3, 1, Duration::from_secs(60));
        let _: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        let _: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert_eq!(cb.failure_count(), 2);

        let result: Result<(), String> = cb.call(|| Ok::<(), &str>(()));
        assert!(result.is_ok());
        assert_eq!(cb.failure_count(), 0);
        assert_eq!(cb.state(), State::Closed);
    }

    #[test]
    fn test_reset() {
        let cb = CircuitBreaker::new(1, 1, Duration::from_secs(60));
        let _: Result<(), String> = cb.call(|| Err::<(), &str>("fail"));
        assert_eq!(cb.state(), State::Open);

        cb.reset();
        assert_eq!(cb.state(), State::Closed);
        assert_eq!(cb.failure_count(), 0);
    }
}
