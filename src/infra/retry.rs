//! Retry with exponential backoff and jitter.
//!
//! Provides a [`retry_with_backoff`] function that wraps a fallible closure and
//! retries it up to a configurable number of times with exponential backoff and
//! random jitter to avoid thundering-herd problems.

use rand::Rng;
use std::time::Duration;

/// Configuration for retry behaviour.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not counting the initial try).
    pub max_retries: u32,
    /// Base delay in milliseconds. Each retry multiplies this by 2.
    pub base_delay_ms: u64,
    /// Maximum delay between retries in milliseconds (cap for exponential
    /// growth).
    pub max_delay_ms: u64,
    /// Whether to add random jitter (±50% of the current delay).
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay_ms: 50,
            max_delay_ms: 5_000,
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration.
    pub const fn new(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            max_retries,
            base_delay_ms,
            max_delay_ms,
            jitter: true,
        }
    }

    /// Execute the closure `f`, retrying on failure with exponential backoff.
    ///
    /// Returns `Ok(T)` on the first success, or the **last** error after all
    /// retries are exhausted.
    ///
    /// The closure receives the current attempt number (0-based).
    pub fn retry_with_backoff<T, E, F>(&self, mut f: F) -> Result<T, E>
    where
        F: FnMut(u32) -> std::result::Result<T, E>,
        E: std::fmt::Display,
    {
        let mut last_err: Option<E> = None;

        for attempt in 0..=self.max_retries {
            match f(attempt) {
                Ok(value) => return Ok(value),
                Err(e) => {
                    if attempt == self.max_retries {
                        return Err(e);
                    }

                    // Log the error for diagnostics.
                    if attempt == 0 {
                        tracing::warn!(
                            target: "apexstore::retry",
                            "Operation failed (attempt {}): {}. Retrying...",
                            attempt + 1,
                            e
                        );
                    } else {
                        tracing::warn!(
                            target: "apexstore::retry",
                            "Operation failed (attempt {} of {}): {}. Retrying...",
                            attempt + 1,
                            self.max_retries + 1,
                            e
                        );
                    }

                    last_err = Some(e);

                    // Calculate delay with exponential backoff.
                    let delay_ms = self.base_delay_ms.saturating_mul(1u64 << attempt);
                    let delay_ms = delay_ms.min(self.max_delay_ms);

                    // Add jitter (±50%) if enabled.
                    let actual_delay_ms = if self.jitter {
                        let half = delay_ms / 2;
                        let min = delay_ms.saturating_sub(half);
                        let max = delay_ms.saturating_add(half);
                        let mut rng = rand::thread_rng();
                        rng.gen_range(min..=max)
                    } else {
                        delay_ms
                    };

                    std::thread::sleep(Duration::from_millis(actual_delay_ms));
                }
            }
        }

        // Unreachable in practice, but the compiler needs it.
        Err(last_err.expect("retry_with_backoff: no error from last attempt"))
    }
}

/// Convenience function that uses [`RetryConfig::default`].
pub fn retry_with_backoff<T, E, F>(f: F) -> Result<T, E>
where
    F: FnMut(u32) -> std::result::Result<T, E>,
    E: std::fmt::Display,
{
    RetryConfig::default().retry_with_backoff(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn test_retry_succeeds_on_first_attempt() {
        let config = RetryConfig::default();
        let result = config.retry_with_backoff(|_| Ok::<_, &str>(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_retry_succeeds_after_retries() {
        let attempts = AtomicU32::new(0);
        let config = RetryConfig::new(3, 5, 100);

        let result = config.retry_with_backoff(|_| {
            let prev = attempts.fetch_add(1, Ordering::SeqCst);
            if prev < 2 {
                Err::<_, &str>("not yet")
            } else {
                Ok("success")
            }
        });

        assert_eq!(result.unwrap(), "success");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_retry_exhausted() {
        let attempts = AtomicU32::new(0);
        let config = RetryConfig::new(2, 5, 100);

        let result: Result<(), &str> = config.retry_with_backoff(|_| {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err("always fails")
        });

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 3); // initial + 2 retries
    }

    #[test]
    fn test_zero_retries() {
        let config = RetryConfig::new(0, 5, 100);
        let result: Result<(), &str> = config.retry_with_backoff(|_| Err("fail"));
        assert!(result.is_err());
    }

    #[test]
    fn test_default_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 50);
        assert_eq!(config.max_delay_ms, 5_000);
        assert!(config.jitter);
    }

    #[test]
    fn test_retry_with_backoff_convenience() {
        let result = retry_with_backoff(|_| Ok::<_, &str>("ok"));
        assert_eq!(result.unwrap(), "ok");
    }
}
