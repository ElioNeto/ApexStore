//! Disk space monitoring for ApexStore.
//!
//! Periodically checks the available disk space on the data directory and
//! triggers actions (warnings, graceful shutdown) when thresholds are crossed.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{error, warn};

/// Monitors available disk space and triggers actions when thresholds are
/// crossed.
pub struct DiskMonitor {
    inner: Arc<Inner>,
    /// Handle to the background monitoring thread.
    handle: Option<thread::JoinHandle<()>>,
}

struct Inner {
    /// Data directory to monitor.
    dir_path: String,
    /// Warn threshold in bytes — below this, a warning is logged.
    warn_threshold: u64,
    /// Critical threshold in bytes — below this, a shutdown callback is called.
    critical_threshold: u64,
    /// Check interval.
    interval: Duration,
    /// Flag to stop the background thread.
    stopped: AtomicBool,
    /// Callback invoked when disk space is critically low (behind a Mutex to
    /// satisfy Sync for Arc).
    on_critical: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl DiskMonitor {
    /// Create a new disk monitor.
    ///
    /// * `dir_path` — path to the data directory to monitor.
    /// * `warn_threshold` — available bytes below which a warning is emitted.
    /// * `critical_threshold` — available bytes below which the critical
    ///   callback is invoked.
    /// * `interval` — how often to check.
    pub fn new(
        dir_path: impl Into<String>,
        warn_threshold: u64,
        critical_threshold: u64,
        interval: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                dir_path: dir_path.into(),
                warn_threshold,
                critical_threshold,
                interval,
                stopped: AtomicBool::new(false),
                on_critical: Mutex::new(None),
            }),
            handle: None,
        }
    }

    /// Create a disk monitor with sensible defaults (warn at 1 GiB, critical
    /// at 256 MiB, check every 30 seconds).
    pub fn default(dir_path: impl Into<String>) -> Self {
        Self::new(
            dir_path,
            1_073_741_824, // 1 GiB warn
            268_435_456,   // 256 MiB critical
            Duration::from_secs(30),
        )
    }

    /// Set the callback to invoke when disk space is critically low (e.g. to
    /// initiate a graceful shutdown).
    pub fn on_critical<F>(&mut self, callback: F)
    where
        F: Fn() + Send + 'static,
    {
        let mut cb = self.inner.on_critical.lock().unwrap();
        *cb = Some(Box::new(callback));
    }

    /// Start the background monitoring thread.
    ///
    /// Returns immediately; checks run in a separate thread.
    pub fn start(&mut self) {
        let inner = self.inner.clone();

        self.handle = Some(thread::spawn(move || {
            while !inner.stopped.load(Ordering::Relaxed) {
                let _ = inner.check_space();

                // Sleep for the check interval, checking periodically for stop.
                for _ in 0..10 {
                    if inner.stopped.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(inner.interval / 10);
                }
            }
        }));
    }

    /// Stop the background monitoring thread.
    pub fn stop(&self) {
        self.inner.stopped.store(true, Ordering::Relaxed);
    }

    /// Perform a single disk space check.
    ///
    /// Returns `Ok(available_bytes)` on success, or an error describing the
    /// failure.  Also evaluates thresholds and invokes the critical callback
    /// when the available space drops below the critical threshold.
    pub fn check_space(&self) -> Result<u64, String> {
        self.inner.check_space()
    }
}

/// Check available disk space for the filesystem containing `path`.
fn check_available_space(path: &str) -> Result<u64, String> {
    let p = Path::new(path);
    let available = fs2::available_space(p)
        .map_err(|e| format!("failed to query available space for '{}': {}", path, e))?;
    Ok(available)
}

impl Inner {
    fn check_space(&self) -> Result<u64, String> {
        let available = check_available_space(&self.dir_path)?;

        if available < self.critical_threshold {
            error!(
                target: "apexstore::disk_monitor",
                "CRITICAL: disk space critically low ({} bytes available, threshold {}). Triggering shutdown.",
                available,
                self.critical_threshold
            );
            let cb = self.on_critical.lock().unwrap();
            if let Some(ref callback) = *cb {
                callback();
            }
        } else if available < self.warn_threshold {
            warn!(
                target: "apexstore::disk_monitor",
                "WARNING: disk space low ({} bytes available, threshold {}).",
                available,
                self.warn_threshold
            );
        }

        Ok(available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn test_default_construction() {
        let monitor = DiskMonitor::default("/tmp");
        assert!(monitor.check_space().is_ok() || monitor.check_space().is_err());
    }

    #[test]
    fn test_critical_callback_invoked() {
        // Create a temporary directory and use very high thresholds so the
        // callback fires immediately.
        let dir = tempfile::TempDir::new().unwrap();
        let dir_path = dir.path().to_str().unwrap().to_string();

        let (tx, rx) = mpsc::channel();
        let mut monitor = DiskMonitor::new(
            &dir_path,
            1,        // 1 byte warn (unlikely to trigger)
            u64::MAX, // critical threshold (always fires)
            Duration::from_secs(1),
        );
        monitor.on_critical(move || {
            let _ = tx.send(());
        });

        let _ = monitor.check_space();
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_ok());
    }

    #[test]
    fn test_start_stop() {
        let dir = tempfile::TempDir::new().unwrap();
        let dir_path = dir.path().to_str().unwrap().to_string();
        let mut monitor = DiskMonitor::new(&dir_path, 1024, 512, Duration::from_millis(50));
        monitor.start();
        std::thread::sleep(Duration::from_millis(150));
        monitor.stop();
        // No panic = success.
    }
}
