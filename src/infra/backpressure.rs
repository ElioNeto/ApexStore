//! Compaction backpressure mechanism.
//!
//! Monitors compaction progress vs write rate and slows down writes when
//! compaction falls behind, preventing unbounded memtable growth and
//! write stalls under heavy load.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::backpressure::CompactionBackpressure;
//!
//! let bp = CompactionBackpressure::default();
//! bp.record_write(1024);
//! bp.record_compaction_progress(512);
//!
//! if bp.should_backpressure() {
//!     let delay = bp.write_delay_ms();
//!     // apply delay before write
//! }
//! ```

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Tracks write and compaction rates to decide when to apply backpressure.
pub struct CompactionBackpressure {
    /// Bytes written since last reset.
    write_bytes: AtomicU64,
    /// Bytes compacted since last reset.
    compacted_bytes: AtomicU64,
    /// Timestamp of the last rate sampling.
    last_sample: Mutex<Instant>,
    /// Write bytes per second (smoothed).
    write_rate_bps: Mutex<f64>,
    /// Compaction bytes per second (smoothed).
    compaction_rate_bps: Mutex<f64>,
    /// Multiplier: how far compaction must lag to trigger backpressure.
    threshold_ratio: f64,
    /// Maximum delay to introduce per write (milliseconds).
    max_delay_ms: u64,
    /// Minimum delay (milliseconds).
    min_delay_ms: u64,
}

impl Default for CompactionBackpressure {
    fn default() -> Self {
        Self {
            write_bytes: AtomicU64::new(0),
            compacted_bytes: AtomicU64::new(0),
            last_sample: Mutex::new(Instant::now()),
            write_rate_bps: Mutex::new(0.0),
            compaction_rate_bps: Mutex::new(0.0),
            threshold_ratio: 2.0, // compaction must keep up with 50% of write rate
            max_delay_ms: 100,
            min_delay_ms: 1,
        }
    }
}

impl CompactionBackpressure {
    /// Create a new backpressure controller with custom thresholds.
    pub fn new(threshold_ratio: f64, max_delay_ms: u64, min_delay_ms: u64) -> Self {
        Self {
            threshold_ratio,
            max_delay_ms,
            min_delay_ms,
            ..Self::default()
        }
    }

    /// Record a write operation of `bytes` bytes.
    pub fn record_write(&self, bytes: u64) {
        self.write_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record compaction progress of `bytes` bytes processed.
    pub fn record_compaction_progress(&self, bytes: u64) {
        self.compacted_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Sample rates and return whether backpressure should be applied.
    ///
    /// Returns `true` when the compaction rate is significantly lower than
    /// the write rate, indicating that compaction cannot keep up.
    pub fn should_backpressure(&self) -> bool {
        self.sample_rates();
        let write_rate = *self.write_rate_bps.lock();
        let compaction_rate = *self.compaction_rate_bps.lock();

        // No writes → no backpressure
        if write_rate < 1.0 {
            return false;
        }

        // Backpressure if compaction rate < write_rate / threshold_ratio
        compaction_rate < write_rate / self.threshold_ratio
    }

    /// Compute the recommended write delay in milliseconds.
    ///
    /// The delay is proportional to how far compaction is behind.
    pub fn write_delay_ms(&self) -> u64 {
        if !self.should_backpressure() {
            return 0;
        }

        let write_rate = *self.write_rate_bps.lock();
        let compaction_rate = *self.compaction_rate_bps.lock();

        if compaction_rate < 1.0 || write_rate < 1.0 {
            return self.min_delay_ms;
        }

        // Delay scales with the ratio of how far behind compaction is
        let ratio = write_rate / compaction_rate;
        let delay = (self.min_delay_ms as f64 * ratio).round() as u64;
        delay.clamp(self.min_delay_ms, self.max_delay_ms)
    }

    /// Reset byte counters and sample rates.
    fn sample_rates(&self) {
        let mut last = self.last_sample.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(*last);
        if elapsed < Duration::from_millis(100) {
            return; // Sample at most 10 times per second
        }

        let secs = elapsed.as_secs_f64().max(0.001);
        let written = self.write_bytes.swap(0, Ordering::Relaxed);
        let compacted = self.compacted_bytes.swap(0, Ordering::Relaxed);

        // Exponential moving average (alpha = 0.3)
        let alpha = 0.3;
        let new_write_rate = written as f64 / secs;
        let new_compact_rate = compacted as f64 / secs;

        let mut wr = self.write_rate_bps.lock();
        *wr = if *wr == 0.0 {
            new_write_rate
        } else {
            alpha * new_write_rate + (1.0 - alpha) * *wr
        };

        let mut cr = self.compaction_rate_bps.lock();
        *cr = if *cr == 0.0 {
            new_compact_rate
        } else {
            alpha * new_compact_rate + (1.0 - alpha) * *cr
        };

        *last = now;
    }

    /// Reset all counters and rate estimates.
    pub fn reset(&self) {
        self.write_bytes.store(0, Ordering::Relaxed);
        self.compacted_bytes.store(0, Ordering::Relaxed);
        *self.last_sample.lock() = Instant::now();
        *self.write_rate_bps.lock() = 0.0;
        *self.compaction_rate_bps.lock() = 0.0;
    }

    /// Get the current write rate (bytes per second, smoothed).
    pub fn write_rate_bps(&self) -> f64 {
        self.sample_rates();
        *self.write_rate_bps.lock()
    }

    /// Get the current compaction rate (bytes per second, smoothed).
    pub fn compaction_rate_bps(&self) -> f64 {
        self.sample_rates();
        *self.compaction_rate_bps.lock()
    }

    /// Get the threshold ratio.
    pub fn threshold_ratio(&self) -> f64 {
        self.threshold_ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_no_backpressure_when_no_writes() {
        let bp = CompactionBackpressure::default();
        assert!(!bp.should_backpressure());
        assert_eq!(bp.write_delay_ms(), 0);
    }

    #[test]
    fn test_backpressure_when_compaction_lags() {
        let bp = CompactionBackpressure::default();
        bp.record_write(10_000);
        bp.record_compaction_progress(1_000);
        // Wait for sample interval
        thread::sleep(Duration::from_millis(150));
        assert!(bp.should_backpressure());
        assert!(bp.write_delay_ms() > 0);
    }

    #[test]
    fn test_no_backpressure_when_compaction_keeps_up() {
        let bp = CompactionBackpressure::default();
        bp.record_write(10_000);
        bp.record_compaction_progress(10_000);
        thread::sleep(Duration::from_millis(150));
        assert!(!bp.should_backpressure());
        assert_eq!(bp.write_delay_ms(), 0);
    }

    #[test]
    fn test_reset() {
        let bp = CompactionBackpressure::default();
        bp.record_write(10_000);
        bp.record_compaction_progress(1_000);
        bp.reset();
        assert_eq!(bp.write_rate_bps(), 0.0);
        assert_eq!(bp.compaction_rate_bps(), 0.0);
    }
}
