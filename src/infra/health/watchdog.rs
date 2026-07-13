//! Watchdog thread for engine health monitoring.
//!
//! A background thread that periodically checks engine health metrics:
//! - WAL write latency exceeding thresholds
//! - Compaction not making progress
//! - Memtable fill rate
//!
//! Logs warnings when health metrics exceed thresholds and provides a
//! snapshot of the current health status.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::health::watchdog::{Watchdog, HealthStatus};
//! use std::time::Duration;
//! use std::sync::Arc;
//!
//! // Create watchdog (requires engine metrics and compaction info)
//! // let watchdog = Watchdog::new(metrics, compaction_progress_fn);
//!
//! // Start monitoring
//! // watchdog.start(Duration::from_secs(5));
//!
//! // Query health
//! // let health = watchdog.last_health();
//!
//! // Stop monitoring
//! // watchdog.stop();
//! ```

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Health status snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    /// Overall health assessment.
    pub healthy: bool,
    /// WAL write latency in microseconds (smoothed).
    pub wal_latency_us: f64,
    /// WAL latency threshold exceeded.
    pub wal_latency_warning: bool,
    /// Compaction making progress (bytes processed per second).
    pub compaction_bytes_per_sec: f64,
    /// Compaction stalled warning.
    pub compaction_stalled: bool,
    /// Memtable fill percentage (0.0 – 1.0).
    pub memtable_fill_ratio: f64,
    /// Memtable near-full warning.
    pub memtable_near_full: bool,
    /// Timestamp of the health check.
    pub checked_at: String,
    /// Number of warnings raised since last reset.
    pub warning_count: u64,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            healthy: true,
            wal_latency_us: 0.0,
            wal_latency_warning: false,
            compaction_bytes_per_sec: 0.0,
            compaction_stalled: false,
            memtable_fill_ratio: 0.0,
            memtable_near_full: false,
            checked_at: chrono::Utc::now().to_rfc3339(),
            warning_count: 0,
        }
    }
}

/// Configuration for the watchdog.
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    /// WAL latency threshold in microseconds (default: 1000 = 1ms).
    pub wal_latency_threshold_us: f64,
    /// Minimum compaction throughput in bytes/sec before warning (default: 1024).
    pub compaction_min_bytes_per_sec: f64,
    /// Memtable fill ratio warning threshold (default: 0.85 = 85%).
    pub memtable_fill_threshold: f64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            wal_latency_threshold_us: 1000.0,
            compaction_min_bytes_per_sec: 1024.0,
            memtable_fill_threshold: 0.85,
        }
    }
}

/// Sampling function types for the watchdog to query engine state.
pub type WalLatencyFn = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type CompactionProgressFn = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type MemtableFillFn = Arc<dyn Fn() -> f64 + Send + Sync>;

/// Shared state for the watchdog thread, protected by Mutex.
struct WatchdogInner {
    running: AtomicBool,
    config: Mutex<WatchdogConfig>,
    last_health: Mutex<HealthStatus>,
    warning_count: Mutex<u64>,
}

/// Watchdog monitor for engine health.
pub struct Watchdog {
    inner: Arc<WatchdogInner>,
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    /// Function to get WAL write latency in microseconds.
    wal_latency_fn: WalLatencyFn,
    /// Function to get compaction progress (bytes/sec).
    compaction_progress_fn: CompactionProgressFn,
    /// Function to get memtable fill ratio (0.0 – 1.0).
    memtable_fill_fn: MemtableFillFn,
}

impl Watchdog {
    /// Create a new watchdog with the given sampling functions.
    ///
    /// * `wal_latency_fn` — returns WAL write latency in microseconds (0.0 if unknown)
    /// * `compaction_progress_fn` — returns compaction throughput in bytes/sec
    /// * `memtable_fill_fn` — returns memtable fill ratio (0.0 – 1.0)
    pub fn new(
        wal_latency_fn: WalLatencyFn,
        compaction_progress_fn: CompactionProgressFn,
        memtable_fill_fn: MemtableFillFn,
    ) -> Self {
        Self {
            inner: Arc::new(WatchdogInner {
                running: AtomicBool::new(false),
                config: Mutex::new(WatchdogConfig::default()),
                last_health: Mutex::new(HealthStatus::default()),
                warning_count: Mutex::new(0),
            }),
            thread_handle: Mutex::new(None),
            wal_latency_fn,
            compaction_progress_fn,
            memtable_fill_fn,
        }
    }

    /// Start the watchdog monitoring thread.
    ///
    /// Polls health metrics every `interval`.
    pub fn start(&self, interval: Duration) {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            tracing::warn!("Watchdog is already running");
            return;
        }

        let inner = self.inner.clone();
        let wal_fn = self.wal_latency_fn.clone();
        let comp_fn = self.compaction_progress_fn.clone();
        let mem_fn = self.memtable_fill_fn.clone();

        let handle = thread::Builder::new()
            .name("watchdog".to_string())
            .spawn(move || {
                // Copy config at start; for live updates, the user must call set_config
                // which updates the Arc. The thread reads config each iteration.
                loop {
                    if !inner.running.load(Ordering::SeqCst) {
                        break;
                    }

                    thread::sleep(interval);

                    let cfg = inner.config.lock();

                    let wal_latency = (wal_fn)();
                    let comp_bytes_sec = (comp_fn)();
                    let mem_fill = (mem_fn)();

                    let wal_warn = wal_latency > cfg.wal_latency_threshold_us;
                    let comp_stalled = comp_bytes_sec < cfg.compaction_min_bytes_per_sec;
                    let mem_full = mem_fill > cfg.memtable_fill_threshold;

                    if wal_warn {
                        *inner.warning_count.lock() += 1;
                        tracing::warn!(
                            "Watchdog: WAL latency high: {:.0}μs (threshold: {:.0}μs)",
                            wal_latency,
                            cfg.wal_latency_threshold_us
                        );
                    }
                    if comp_stalled {
                        *inner.warning_count.lock() += 1;
                        tracing::warn!(
                            "Watchdog: Compaction stalled: {:.0} bytes/sec (min: {:.0})",
                            comp_bytes_sec,
                            cfg.compaction_min_bytes_per_sec
                        );
                    }
                    if mem_full {
                        *inner.warning_count.lock() += 1;
                        tracing::warn!(
                            "Watchdog: Memtable near full: {:.1}% (threshold: {:.1}%)",
                            mem_fill * 100.0,
                            cfg.memtable_fill_threshold * 100.0
                        );
                    }

                    drop(cfg);

                    let health = HealthStatus {
                        healthy: !wal_warn && !comp_stalled && !mem_full,
                        wal_latency_us: wal_latency,
                        wal_latency_warning: wal_warn,
                        compaction_bytes_per_sec: comp_bytes_sec,
                        compaction_stalled: comp_stalled,
                        memtable_fill_ratio: mem_fill,
                        memtable_near_full: mem_full,
                        checked_at: chrono::Utc::now().to_rfc3339(),
                        warning_count: *inner.warning_count.lock(),
                    };

                    *inner.last_health.lock() = health;
                }
            })
            .expect("Failed to spawn watchdog thread");

        *self.thread_handle.lock() = Some(handle);
    }

    /// Stop the watchdog monitoring thread.
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.thread_handle.lock().take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }

    /// Get the last recorded health status.
    pub fn last_health(&self) -> HealthStatus {
        self.inner.last_health.lock().clone()
    }

    /// Update watchdog configuration.
    ///
    /// Note: configuration changes take effect on the next health check cycle.
    pub fn set_config(&self, config: WatchdogConfig) {
        *self.inner.config.lock() = config;
    }

    /// Reset the warning counter.
    pub fn reset_warnings(&self) {
        *self.inner.warning_count.lock() = 0;
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_health() {
        let wal_fn = Arc::new(|| 0.0f64) as WalLatencyFn;
        let comp_fn = Arc::new(|| 0.0f64) as CompactionProgressFn;
        let mem_fn = Arc::new(|| 0.0f64) as MemtableFillFn;

        let wd = Watchdog::new(wal_fn, comp_fn, mem_fn);
        let health = wd.last_health();
        assert!(health.healthy);
        assert_eq!(health.warning_count, 0);
    }

    #[test]
    fn test_health_check() {
        let wal_fn = Arc::new(|| 2000.0f64) as WalLatencyFn;
        let comp_fn = Arc::new(|| 100.0f64) as CompactionProgressFn;
        let mem_fn = Arc::new(|| 0.9f64) as MemtableFillFn;

        let _wd = Watchdog::new(wal_fn.clone(), comp_fn.clone(), mem_fn.clone());

        let cfg = WatchdogConfig::default();
        let wal_warn = (wal_fn)() > cfg.wal_latency_threshold_us;
        let comp_stalled = (comp_fn)() < cfg.compaction_min_bytes_per_sec;
        let mem_full = (mem_fn)() > cfg.memtable_fill_threshold;

        assert!(wal_warn);
        assert!(comp_stalled);
        assert!(mem_full);
    }

    #[test]
    fn test_set_config() {
        let wal_fn = Arc::new(|| 0.0f64) as WalLatencyFn;
        let comp_fn = Arc::new(|| 0.0f64) as CompactionProgressFn;
        let mem_fn = Arc::new(|| 0.0f64) as MemtableFillFn;

        let wd = Watchdog::new(wal_fn, comp_fn, mem_fn);
        wd.set_config(WatchdogConfig {
            wal_latency_threshold_us: 500.0,
            compaction_min_bytes_per_sec: 512.0,
            memtable_fill_threshold: 0.9,
        });
    }
}
