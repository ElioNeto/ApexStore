//! Memory limit enforcement for ApexStore.
//!
//! Tracks approximate memory usage across memtables, block cache, and WAL
//! buffers. Provides a budgeting mechanism so callers can request allocations
//! and be denied when the limit would be exceeded.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Tracks approximate memory usage and enforces a configurable limit.
///
/// Use [`try_allocate`](MemoryLimiter::try_allocate) to request memory before
/// performing an allocation, and [`release`](MemoryLimiter::release) when the
/// memory is freed. Callers should treat a denied allocation as a signal to
/// flush memtables, evict cache entries, or return a back-pressure error.
pub struct MemoryLimiter {
    /// Maximum allowed usage in bytes.
    limit: usize,
    /// Current tracked usage in bytes.
    current: AtomicUsize,
    /// Peak usage observed (for diagnostics).
    peak: AtomicUsize,
}

impl MemoryLimiter {
    /// Create a new memory limiter with the given byte limit.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// Try to reserve `bytes` of memory.
    ///
    /// Returns `true` if the allocation would keep total usage below the limit;
    /// returns `false` if the budget is exhausted.
    ///
    /// The caller MUST call [`release`](MemoryLimiter::release) with the same
    /// amount when the memory is freed, otherwise the budget will leak.
    pub fn try_allocate(&self, bytes: usize) -> bool {
        loop {
            let current = self.current.load(Ordering::Relaxed);
            let new = current + bytes;
            if new > self.limit {
                return false;
            }
            if self
                .current
                .compare_exchange(current, new, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                // Update peak (best-effort, not critical for correctness)
                let _ = self.peak.fetch_max(new, Ordering::Relaxed);
                return true;
            }
        }
    }

    /// Release `bytes` of previously allocated memory.
    pub fn release(&self, bytes: usize) {
        // Saturating subtraction — if we somehow release more than allocated,
        // just go to zero rather than wrapping around.
        let _ = self
            .current
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |c| {
                Some(c.saturating_sub(bytes))
            });
    }

    /// Returns the current tracked memory usage in bytes.
    pub fn usage(&self) -> usize {
        self.current.load(Ordering::Relaxed)
    }

    /// Returns the configured memory limit in bytes.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the fraction of memory used (`0.0` to `1.0`).
    pub fn usage_ratio(&self) -> f64 {
        if self.limit == 0 {
            return 0.0;
        }
        self.usage() as f64 / self.limit as f64
    }

    /// Returns peak usage observed.
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }

    /// Reset current usage to zero (e.g. after a full flush).
    pub fn reset(&self) {
        self.current.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_within_limit() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(50));
        assert_eq!(limiter.usage(), 50);
        assert_eq!(limiter.limit(), 100);
    }

    #[test]
    fn test_allocate_exceeds_limit() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(60));
        assert!(!limiter.try_allocate(50)); // would exceed
        assert_eq!(limiter.usage(), 60);
    }

    #[test]
    fn test_release() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(80));
        assert_eq!(limiter.usage(), 80);
        limiter.release(30);
        assert_eq!(limiter.usage(), 50);
        limiter.release(50);
        assert_eq!(limiter.usage(), 0);
    }

    #[test]
    fn test_release_saturating() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(10));
        limiter.release(100); // more than allocated
        assert_eq!(limiter.usage(), 0); // saturates at 0
    }

    #[test]
    fn test_peak() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(30));
        assert!(limiter.try_allocate(40));
        assert_eq!(limiter.peak(), 70);
        limiter.release(70);
        assert_eq!(limiter.usage(), 0);
        assert_eq!(limiter.peak(), 70); // peak is not reset
    }

    #[test]
    fn test_reset() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(80));
        assert_eq!(limiter.usage(), 80);
        limiter.reset();
        assert_eq!(limiter.usage(), 0);
    }

    #[test]
    fn test_usage_ratio() {
        let limiter = MemoryLimiter::new(100);
        assert!(limiter.try_allocate(25));
        assert!((limiter.usage_ratio() - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_zero_limit() {
        let limiter = MemoryLimiter::new(0);
        assert!(!limiter.try_allocate(1));
        assert_eq!(limiter.usage_ratio(), 0.0);
    }
}
