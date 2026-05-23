//! Chaos testing framework.
//!
//! Only enabled in test/dev builds (`#[cfg(feature = "chaos")]`).
//! Provides failure injection for:
//! - Disk latency
//! - Disk full simulation
//! - Compaction panics (probabilistic)
//! - WAL fsync kills
//! - SSTable corruption
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::chaos::{ChaosEngine, FailureType};
//! use std::time::Duration;
//!
//! let chaos = ChaosEngine::new();
//!
//! // Inject disk latency
//! chaos.inject(FailureType::DiskLatency {
//!     duration: Duration::from_secs(10),
//!     delay: Duration::from_millis(200),
//! });
//!
//! // List active experiments
//! let active = chaos.list_active();
//!
//! // Stop an experiment by ID
//! // chaos.stop("experiment-id");
//! ```

use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Types of failures that can be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureType {
    /// Inject artificial delay on disk I/O operations.
    DiskLatency {
        /// How long the experiment runs.
        duration: Duration,
        /// Additional delay per I/O operation.
        delay: Duration,
    },
    /// Simulate a full disk by failing writes with "no space left" errors.
    DiskFull {
        /// How long the experiment runs.
        duration: Duration,
        /// Apparent capacity limit in bytes.
        size: u64,
    },
    /// Probabilistically panic during compaction.
    PanicCompaction {
        /// Probability (0.0 – 1.0) of panicking per compaction cycle.
        probability: f64,
    },
    /// Kill WAL fsync (fsync appears to succeed but data is not persisted).
    KillWalFsync,
    /// Corrupt SSTable data on write.
    CorruptSstable {
        /// Probability (0.0 – 1.0) of corrupting a block on write.
        probability: f64,
    },
}

/// Status of an active chaos experiment.
#[derive(Debug, Clone, Serialize)]
pub struct ExperimentStatus {
    /// Unique experiment ID.
    pub id: String,
    /// Type of failure being injected.
    pub failure_type: FailureType,
    /// When the experiment was started.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Whether the experiment is still active.
    pub active: bool,
}

/// Manages chaos experiments for failure injection.
pub struct ChaosEngine {
    /// Active experiments.
    experiments: Mutex<HashMap<String, ExperimentStatus>>,
    /// Whether chaos mode is enabled globally.
    enabled: AtomicBool,
    /// Disk I/O delay override (set by DiskLatency experiment).
    pub(crate) disk_delay: Mutex<Option<Duration>>,
    /// Disk full limit override (set by DiskFull experiment).
    pub(crate) disk_full_limit: Mutex<Option<u64>>,
    /// Compaction panic probability (set by PanicCompaction experiment).
    pub(crate) compaction_panic_prob: Mutex<f64>,
    /// Corrupt SSTable probability (set by CorruptSstable experiment).
    pub(crate) corrupt_sstable_prob: Mutex<f64>,
    /// Kill WAL fsync flag (set by KillWalFsync experiment).
    pub(crate) kill_wal_fsync: AtomicBool,
    /// Total bytes written during DiskFull simulation.
    pub(crate) bytes_written: AtomicU64,
}

impl Default for ChaosEngine {
    fn default() -> Self {
        Self {
            experiments: Mutex::new(HashMap::new()),
            enabled: AtomicBool::new(cfg!(feature = "chaos")),
            disk_delay: Mutex::new(None),
            disk_full_limit: Mutex::new(None),
            compaction_panic_prob: Mutex::new(0.0),
            corrupt_sstable_prob: Mutex::new(0.0),
            kill_wal_fsync: AtomicBool::new(false),
            bytes_written: AtomicU64::new(0),
        }
    }
}

impl ChaosEngine {
    /// Create a new `ChaosEngine`.
    ///
    /// Chaos is only enabled when the `chaos` feature is active.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject a failure of the given type.
    ///
    /// Returns a unique experiment ID that can be used to stop the experiment.
    pub fn inject(&self, failure_type: FailureType) -> String {
        if !self.enabled.load(Ordering::Relaxed) {
            tracing::warn!("Chaos engine is not enabled (compile with --features chaos)");
            return String::new();
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // Apply the failure mode
        match &failure_type {
            FailureType::DiskLatency { duration: _, delay } => {
                *self.disk_delay.lock() = Some(*delay);
                tracing::info!("Chaos: injected DiskLatency (delay: {:?})", delay);
            }
            FailureType::DiskFull { duration: _, size } => {
                *self.disk_full_limit.lock() = Some(*size);
                tracing::info!("Chaos: injected DiskFull (size limit: {})", size);
            }
            FailureType::PanicCompaction { probability } => {
                *self.compaction_panic_prob.lock() = *probability;
                tracing::info!("Chaos: injected PanicCompaction (p={})", probability);
            }
            FailureType::KillWalFsync => {
                self.kill_wal_fsync.store(true, Ordering::Relaxed);
                tracing::info!("Chaos: injected KillWalFsync");
            }
            FailureType::CorruptSstable { probability } => {
                *self.corrupt_sstable_prob.lock() = *probability;
                tracing::info!("Chaos: injected CorruptSstable (p={})", probability);
            }
        }

        let status = ExperimentStatus {
            id: id.clone(),
            failure_type,
            started_at: now,
            active: true,
        };

        self.experiments.lock().insert(id.clone(), status);
        id
    }

    /// List all active experiments.
    pub fn list_active(&self) -> Vec<ExperimentStatus> {
        self.experiments
            .lock()
            .values()
            .filter(|e| e.active)
            .cloned()
            .collect()
    }

    /// Stop a specific experiment by ID.
    ///
    /// Reverses the failure mode that was injected.
    pub fn stop(&self, experiment_id: &str) -> bool {
        let mut experiments = self.experiments.lock();
        if let Some(status) = experiments.get(experiment_id) {
            if !status.active {
                return false;
            }
            // Reverse the failure mode
            match &status.failure_type {
                FailureType::DiskLatency { .. } => {
                    *self.disk_delay.lock() = None;
                }
                FailureType::DiskFull { .. } => {
                    *self.disk_full_limit.lock() = None;
                }
                FailureType::PanicCompaction { .. } => {
                    *self.compaction_panic_prob.lock() = 0.0;
                }
                FailureType::KillWalFsync => {
                    self.kill_wal_fsync.store(false, Ordering::Relaxed);
                }
                FailureType::CorruptSstable { .. } => {
                    *self.corrupt_sstable_prob.lock() = 0.0;
                }
            }
            if let Some(status) = experiments.get_mut(experiment_id) {
                status.active = false;
            }
            tracing::info!("Chaos: stopped experiment {}", experiment_id);
            true
        } else {
            false
        }
    }

    /// Stop all active experiments.
    pub fn stop_all(&self) {
        let ids: Vec<String> = self
            .experiments
            .lock()
            .iter()
            .filter(|(_, s)| s.active)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.stop(&id);
        }
    }

    /// Check if chaos mode is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enable or disable chaos mode.
    ///
    /// When disabled, injected failures are ignored.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            self.stop_all();
        }
    }

    /// Inject disk latency for the given duration.
    ///
    /// Convenience wrapper around `inject(FailureType::DiskLatency { ... })`.
    pub fn inject_disk_latency(&self, duration: Duration, delay: Duration) -> String {
        self.inject(FailureType::DiskLatency { duration, delay })
    }

    /// Simulate a full disk with the given size limit.
    pub fn simulate_disk_full(&self, size: u64) -> String {
        self.inject(FailureType::DiskFull {
            duration: Duration::from_secs(30),
            size,
        })
    }

    /// Set compaction panic probability.
    pub fn panic_compaction(&self, probability: f64) -> String {
        self.inject(FailureType::PanicCompaction { probability })
    }

    /// Get the current disk I/O delay (if any).
    pub fn current_disk_delay(&self) -> Option<Duration> {
        *self.disk_delay.lock()
    }

    /// Get the current disk full limit (if any).
    pub fn current_disk_full_limit(&self) -> Option<u64> {
        *self.disk_full_limit.lock()
    }

    /// Check if WAL fsync should be skipped.
    pub fn should_kill_fsync(&self) -> bool {
        self.kill_wal_fsync.load(Ordering::Relaxed)
    }

    /// Get the current SSTable corruption probability.
    pub fn corrupt_probability(&self) -> f64 {
        *self.corrupt_sstable_prob.lock()
    }

    /// Get the current compaction panic probability.
    pub fn compaction_panic_probability(&self) -> f64 {
        *self.compaction_panic_prob.lock()
    }

    /// Check whether writing `additional_bytes` would exceed the disk full limit.
    ///
    /// Returns `true` if a `DiskFull` experiment is active and the total
    /// written bytes plus `additional_bytes` exceeds the limit.
    pub fn check_disk_full(&self, additional_bytes: u64) -> bool {
        if !self.enabled.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(limit) = *self.disk_full_limit.lock() {
            let written = self.bytes_written.load(Ordering::Relaxed);
            let would_exceed = written.saturating_add(additional_bytes) > limit;
            if would_exceed {
                return true;
            }
            // Track the bytes as written
            self.bytes_written
                .fetch_add(additional_bytes, Ordering::Relaxed);
        }
        false
    }

    /// Return the current disk I/O delay, if any.
    ///
    /// Returns `Some(delay)` when a `DiskLatency` experiment is active.
    pub fn should_delay_io(&self) -> Option<Duration> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        *self.disk_delay.lock()
    }

    /// Check whether the current write should be corrupted, based on the
    /// `CorruptSstable` experiment probability.
    pub fn should_corrupt_write(&self) -> bool {
        if !self.enabled.load(Ordering::Relaxed) {
            return false;
        }
        let prob = *self.corrupt_sstable_prob.lock();
        if prob <= 0.0 {
            return false;
        }
        let mut rng = rand::thread_rng();
        rng.gen::<f64>() < prob
    }
}

// ── Global singleton access ───────────────────────────────────────────────────

/// Global `ChaosEngine` singleton, lazily initialised on first access.
static CHAOS_ENGINE: OnceLock<ChaosEngine> = OnceLock::new();

/// Initialise the global `ChaosEngine` singleton.
///
/// If the singleton has already been initialised, this is a no-op.
pub fn init_global() {
    CHAOS_ENGINE.get_or_init(|| {
        let engine = ChaosEngine::new();
        if cfg!(feature = "chaos") {
            engine.set_enabled(true);
        }
        engine
    });
}

/// Return a reference to the global `ChaosEngine` singleton.
///
/// # Panics
/// Panics if [`init_global`] has not been called first.
pub fn global() -> &'static ChaosEngine {
    CHAOS_ENGINE
        .get()
        .expect("ChaosEngine global not initialised — call init_global() first")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_and_stop() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        let id = chaos.inject(FailureType::DiskLatency {
            duration: Duration::from_secs(10),
            delay: Duration::from_millis(100),
        });

        assert!(!id.is_empty());
        assert_eq!(chaos.list_active().len(), 1);
        assert!(chaos.current_disk_delay().is_some());

        assert!(chaos.stop(&id));
        assert_eq!(chaos.list_active().len(), 0);
        assert!(chaos.current_disk_delay().is_none());
    }

    #[test]
    fn test_inject_disk_latency() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        chaos.inject_disk_latency(Duration::from_secs(5), Duration::from_millis(200));
        assert_eq!(chaos.current_disk_delay(), Some(Duration::from_millis(200)));
    }

    #[test]
    fn test_simulate_disk_full() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        chaos.simulate_disk_full(1024);
        assert_eq!(chaos.current_disk_full_limit(), Some(1024));
    }

    #[test]
    fn test_panic_compaction() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        chaos.panic_compaction(0.5);
        assert!((chaos.compaction_panic_probability() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_kill_wal_fsync() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        chaos.inject(FailureType::KillWalFsync);
        assert!(chaos.should_kill_fsync());

        chaos.stop_all();
        assert!(!chaos.should_kill_fsync());
    }

    #[test]
    fn test_stop_nonexistent() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        assert!(!chaos.stop("nonexistent-id"));
    }

    #[test]
    fn test_corrupt_sstable() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);

        chaos.inject(FailureType::CorruptSstable { probability: 0.1 });
        assert!((chaos.corrupt_probability() - 0.1).abs() < f64::EPSILON);
    }

    // ── Hook method tests ──────────────────────────────────────────────────

    #[test]
    fn test_check_disk_full_not_enabled() {
        let chaos = ChaosEngine::new();
        // disabled by default in non-chaos builds
        assert!(!chaos.check_disk_full(100));
    }

    #[test]
    fn test_check_disk_full_within_limit() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        chaos.simulate_disk_full(1024);

        // Writing 500 bytes should be fine
        assert!(!chaos.check_disk_full(500));
    }

    #[test]
    fn test_check_disk_full_exceeds_limit() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        chaos.simulate_disk_full(600);

        // First write of 400 bytes — within limit
        assert!(!chaos.check_disk_full(400));
        // Second write of 300 bytes — exceeds the remaining 200
        assert!(chaos.check_disk_full(300));
    }

    #[test]
    fn test_should_delay_io_none() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        assert!(chaos.should_delay_io().is_none());
    }

    #[test]
    fn test_should_delay_io_some() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        chaos.inject_disk_latency(Duration::from_secs(10), Duration::from_millis(50));
        assert_eq!(chaos.should_delay_io(), Some(Duration::from_millis(50)));
    }

    #[test]
    fn test_should_delay_io_disabled() {
        let chaos = ChaosEngine::new();
        // disabled — should return None regardless of injected state
        assert!(chaos.should_delay_io().is_none());
    }

    #[test]
    fn test_should_corrupt_write_zero_prob() {
        let chaos = ChaosEngine::new();
        chaos.set_enabled(true);
        // Probability is 0.0 by default
        assert!(!chaos.should_corrupt_write());
    }

    #[test]
    fn test_global_init_and_access() {
        // Reset for test isolation — OnceLock can't be reset, so we just
        // verify that init_global() and global() don't panic.
        init_global();
        let g = global();
        assert!(!g.is_enabled() || cfg!(feature = "chaos"));
    }
}
