//! Panic recovery for worker threads.
//!
//! Wraps thread spawns with `std::panic::catch_unwind` so that panics in
//! worker threads (compaction, background I/O) are caught, logged, and the
//! thread can be restarted. Maintains a history of recent panics for
//! observability.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::panic_recovery::PanicRecovery;
//!
//! let recovery = PanicRecovery::new();
//!
//! // Spawn a protected thread
//! let handle = recovery.spawn_protected(|| {
//!     // worker logic that might panic
//! });
//!
//! // Register a callback for panic events
//! recovery.on_panic(Box::new(|info| {
//!     eprintln!("Thread panicked: {}", info.reason);
//! }));
//! ```

use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

/// Type alias for the panic callback.
type PanicCallback = Box<dyn Fn(&PanicInfo) + Send + Sync>;

/// Information about a captured panic.
#[derive(Debug, Clone)]
pub struct PanicInfo {
    /// Human-readable panic reason.
    pub reason: String,
    /// Timestamp (Unix epoch nanos) when the panic occurred.
    pub occurred_at: u64,
    /// Name of the thread that panicked, if available.
    pub thread_name: Option<String>,
}

/// Manages panic recovery for worker threads.
///
/// Wraps `thread::spawn` with `std::panic::catch_unwind` so that panics
/// are captured instead of crashing the process.
pub struct PanicRecovery {
    /// Recent panic history (circular buffer) — shared via Arc so spawned
    /// threads can record panics on the same instance.
    panics: Arc<Mutex<Vec<PanicInfo>>>,
    /// Maximum number of recent panics to retain.
    max_history: usize,
    /// Callback invoked on each panic — shared via Arc so spawned threads
    /// can invoke the same callback.
    on_panic_callback: Arc<Mutex<Option<PanicCallback>>>,
}

impl Default for PanicRecovery {
    fn default() -> Self {
        Self {
            panics: Arc::new(Mutex::new(Vec::with_capacity(16))),
            max_history: 16,
            on_panic_callback: Arc::new(Mutex::new(None)),
        }
    }
}

impl PanicRecovery {
    /// Create a new `PanicRecovery` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a thread with panic protection.
    ///
    /// If the closure panics, the panic is caught, recorded, and the
    /// registered callback (if any) is invoked. The `JoinHandle` will
    /// still return normally (no panic propagation).
    pub fn spawn_protected<F, T>(&self, name: Option<&str>, f: F) -> JoinHandle<Option<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let recovery = Arc::new(self.clone_inner());
        let thread_name = name.unwrap_or("unnamed").to_string();

        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
                match result {
                    Ok(val) => Some(val),
                    Err(payload) => {
                        let info = PanicRecovery::extract_panic_info(&payload, &thread_name);
                        recovery.record_panic(info.clone());
                        recovery.invoke_callback(&info);
                        None
                    }
                }
            })
            .expect("Failed to spawn protected thread")
    }

    /// Register a callback that is invoked on every panic.
    pub fn on_panic(&self, callback: Box<dyn Fn(&PanicInfo) + Send + Sync>) {
        *self.on_panic_callback.lock() = Some(callback);
    }

    /// Return a copy of recent panics.
    pub fn recent_panics(&self) -> Vec<PanicInfo> {
        self.panics.lock().clone()
    }

    /// Clear the panic history.
    pub fn clear_history(&self) {
        self.panics.lock().clear();
    }

    // ── Internal helpers ──

    /// Create a clone of self internals for use in spawned threads.
    ///
    /// The returned instance shares the same `panics` buffer and
    /// `on_panic_callback` via `Arc`, so panics in spawned threads are
    /// visible on the original `PanicRecovery`.
    fn clone_inner(&self) -> Self {
        Self {
            panics: self.panics.clone(),
            max_history: self.max_history,
            on_panic_callback: self.on_panic_callback.clone(),
        }
    }

    /// Extract panic info from a `Box<dyn Any>` payload.
    fn extract_panic_info(payload: &Box<dyn Any + Send>, thread_name: &str) -> PanicInfo {
        let reason = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            format!("panic: {:?}", payload)
        };

        let occurred_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;

        PanicInfo {
            reason,
            occurred_at,
            thread_name: Some(thread_name.to_string()),
        }
    }

    /// Record a panic in the history buffer.
    fn record_panic(&self, info: PanicInfo) {
        let mut panics = self.panics.lock();
        panics.push(info);
        if panics.len() > self.max_history {
            panics.remove(0);
        }
    }

    /// Invoke the registered panic callback.
    fn invoke_callback(&self, info: &PanicInfo) {
        if let Some(ref callback) = *self.on_panic_callback.lock() {
            callback(info);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_spawn_protected_no_panic() {
        let recovery = PanicRecovery::new();
        let handle = recovery.spawn_protected(Some("test"), || 42);
        let result = handle.join().unwrap();
        assert_eq!(result, Some(42));
        assert!(recovery.recent_panics().is_empty());
    }

    #[test]
    fn test_spawn_protected_catches_panic() {
        let recovery = PanicRecovery::new();

        let handle = recovery.spawn_protected(Some("panic_test"), || {
            panic!("intentional panic for test");
        });
        let result = handle.join().unwrap();
        assert!(result.is_none());

        let panics = recovery.recent_panics();
        assert!(!panics.is_empty());
        assert!(panics[0].reason.contains("intentional panic for test"));
    }

    #[test]
    fn test_on_panic_callback() {
        let recovery = PanicRecovery::new();
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_clone = invoked.clone();

        recovery.on_panic(Box::new(move |_info| {
            invoked_clone.store(true, Ordering::SeqCst);
        }));

        let handle = recovery.spawn_protected(Some("callback_test"), || {
            panic!("another intentional panic");
        });
        let _ = handle.join();
        std::thread::sleep(Duration::from_millis(50));

        assert!(invoked.load(Ordering::SeqCst));
    }

    #[test]
    fn test_clear_history() {
        let recovery = PanicRecovery::new();
        let handle = recovery.spawn_protected(Some("clear_test"), || {
            panic!("panic for clear test");
        });
        let _ = handle.join();
        assert!(!recovery.recent_panics().is_empty());

        recovery.clear_history();
        assert!(recovery.recent_panics().is_empty());
    }
}
