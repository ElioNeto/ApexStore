use crate::infra::telemetry::OtelInstruments;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A point-in-time snapshot of engine metrics, serializable to JSON.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub sets: u64,
    pub gets: u64,
    pub deletes: u64,
    pub scans: u64,
    pub batch_sets: u64,
    pub batch_deletes: u64,
    pub flushes: u64,
    pub compactions: u64,
    pub set_latency_us: u64,
    pub get_latency_us: u64,
    pub delete_latency_us: u64,
    pub scan_latency_us: u64,
    pub flush_latency_us: u64,
    pub compaction_latency_us: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub bloom_filter_negatives: u64,
    pub errors: u64,
}

/// Atomic counters and latency accumulators for the engine.
///
/// All fields are `AtomicU64` so they can be updated concurrently without locks.
/// Latency is accumulated in microseconds; clients can compute average latency
/// by dividing `*_latency_us` by the corresponding operation counter.
#[derive(Debug, Default)]
pub struct EngineMetrics {
    // Operation counters
    pub sets: AtomicU64,
    pub gets: AtomicU64,
    pub deletes: AtomicU64,
    pub scans: AtomicU64,
    pub batch_sets: AtomicU64,
    pub batch_deletes: AtomicU64,
    pub flushes: AtomicU64,
    pub compactions: AtomicU64,

    // Latency accumulators (in microseconds)
    pub set_latency_us: AtomicU64,
    pub get_latency_us: AtomicU64,
    pub delete_latency_us: AtomicU64,
    pub scan_latency_us: AtomicU64,
    pub flush_latency_us: AtomicU64,
    pub compaction_latency_us: AtomicU64,

    // Cache metrics
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub bloom_filter_negatives: AtomicU64,

    // Error counter
    pub errors: AtomicU64,

    // Write amplification tracking
    /// Total bytes written to SSTables (via flush + compaction).
    pub total_sstable_bytes_written: AtomicU64,
    /// Total bytes written to WAL.
    pub total_wal_bytes_written: AtomicU64,
    /// Total bytes read from SSTables.
    pub total_sstable_bytes_read: AtomicU64,

    /// Optional OpenTelemetry instruments for exporting metrics via OTLP.
    /// When `Some`, every `record_*` call also updates the corresponding OTel counter.
    pub otel_instruments: Option<Arc<OtelInstruments>>,
}

impl EngineMetrics {
    /// Create a new `EngineMetrics` with all counters initialised to zero.
    pub fn new() -> Self {
        Self {
            otel_instruments: None,
            ..Self::default()
        }
    }

    /// Attach an OTel instruments handle so that record methods also
    /// export metrics via the OpenTelemetry OTLP pipeline.
    pub fn set_otel_instruments(&mut self, instruments: Option<Arc<OtelInstruments>>) {
        self.otel_instruments = instruments;
    }

    // ── Record helpers (counter + latency) ──

    #[inline]
    pub fn record_set(&self, duration_us: u64) {
        self.sets.fetch_add(1, Ordering::Relaxed);
        self.set_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.sets.add(1, &[]);
            inst.set_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_get(&self, duration_us: u64) {
        self.gets.fetch_add(1, Ordering::Relaxed);
        self.get_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.gets.add(1, &[]);
            inst.get_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_delete(&self, duration_us: u64) {
        self.deletes.fetch_add(1, Ordering::Relaxed);
        self.delete_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.deletes.add(1, &[]);
            inst.delete_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_scan(&self, duration_us: u64) {
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.scan_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.scans.add(1, &[]);
            inst.scan_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_batch_sets(&self, count: u64) {
        self.batch_sets.fetch_add(count, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.batch_sets.add(count, &[]);
        }
    }

    #[inline]
    pub fn record_batch_deletes(&self, count: u64) {
        self.batch_deletes.fetch_add(count, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.batch_deletes.add(count, &[]);
        }
    }

    #[inline]
    pub fn record_flush(&self, duration_us: u64) {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        self.flush_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.flushes.add(1, &[]);
            inst.flush_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_compaction(&self, duration_us: u64) {
        self.compactions.fetch_add(1, Ordering::Relaxed);
        self.compaction_latency_us
            .fetch_add(duration_us, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.compactions.add(1, &[]);
            inst.compaction_latency.add(duration_us, &[]);
        }
    }

    #[inline]
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.cache_hits.add(1, &[]);
        }
    }

    #[inline]
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.cache_misses.add(1, &[]);
        }
    }

    #[inline]
    pub fn record_bloom_negative(&self) {
        self.bloom_filter_negatives.fetch_add(1, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.bloom_negatives.add(1, &[]);
        }
    }

    #[inline]
    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        if let Some(ref inst) = self.otel_instruments {
            inst.errors.add(1, &[]);
        }
    }

    // ── Write amplification tracking ──

    #[inline]
    pub fn record_sstable_write(&self, bytes: u64) {
        self.total_sstable_bytes_written
            .fetch_add(bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_wal_write(&self, bytes: u64) {
        self.total_wal_bytes_written
            .fetch_add(bytes, Ordering::Relaxed);
    }

    #[inline]
    pub fn record_sstable_read(&self, bytes: u64) {
        self.total_sstable_bytes_read
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Calculate write amplification factor:
    /// (SST bytes written + WAL bytes written) / max(SST bytes read, 1)
    pub fn write_amplification(&self) -> f64 {
        let written = self.total_sstable_bytes_written.load(Ordering::Relaxed) as f64
            + self.total_wal_bytes_written.load(Ordering::Relaxed) as f64;
        let read = self.total_sstable_bytes_read.load(Ordering::Relaxed).max(1) as f64;
        written / read
    }

    /// Calculate read amplification: SSTable bytes read per user operation.
    /// (total SST bytes read) / max(user gets + scans, 1)
    pub fn read_amplification(&self) -> f64 {
        let sstable_reads = self.total_sstable_bytes_read.load(Ordering::Relaxed) as f64;
        let user_ops =
            (self.gets.load(Ordering::Relaxed) + self.scans.load(Ordering::Relaxed)).max(1) as f64;
        sstable_reads / user_ops
    }

    // ── Snapshot ──

    /// Atomically snapshot all counters into a JSON-serializable struct.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            sets: self.sets.load(Ordering::Relaxed),
            gets: self.gets.load(Ordering::Relaxed),
            deletes: self.deletes.load(Ordering::Relaxed),
            scans: self.scans.load(Ordering::Relaxed),
            batch_sets: self.batch_sets.load(Ordering::Relaxed),
            batch_deletes: self.batch_deletes.load(Ordering::Relaxed),
            flushes: self.flushes.load(Ordering::Relaxed),
            compactions: self.compactions.load(Ordering::Relaxed),
            set_latency_us: self.set_latency_us.load(Ordering::Relaxed),
            get_latency_us: self.get_latency_us.load(Ordering::Relaxed),
            delete_latency_us: self.delete_latency_us.load(Ordering::Relaxed),
            scan_latency_us: self.scan_latency_us.load(Ordering::Relaxed),
            flush_latency_us: self.flush_latency_us.load(Ordering::Relaxed),
            compaction_latency_us: self.compaction_latency_us.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            bloom_filter_negatives: self.bloom_filter_negatives.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }

    // ── Prometheus text format ──

    /// Format all metrics in Prometheus exposition format.
    ///
    /// See <https://prometheus.io/docs/instrumenting/exposition_formats/>
    pub fn format_prometheus(&self) -> String {
        let mut out = String::new();

        // Helper macro to emit HELP, TYPE and value lines
        macro_rules! prom_counter {
            ($name:expr, $help:expr, $val:expr) => {{
                out.push_str("# HELP ");
                out.push_str($name);
                out.push(' ');
                out.push_str($help);
                out.push('\n');
                out.push_str("# TYPE ");
                out.push_str($name);
                out.push_str(" counter\n");
                out.push_str($name);
                out.push(' ');
                out.push_str(&itoa_dispatch($val));
                out.push('\n');
            }};
        }

        prom_counter!(
            "apexstore_sets_total",
            "Total number of set operations",
            self.sets.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_gets_total",
            "Total number of get operations",
            self.gets.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_deletes_total",
            "Total number of delete operations",
            self.deletes.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_scans_total",
            "Total number of scan operations",
            self.scans.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_batch_sets_total",
            "Total number of items in batch set operations",
            self.batch_sets.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_batch_deletes_total",
            "Total number of items in batch delete operations",
            self.batch_deletes.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_flushes_total",
            "Total number of memtable flushes",
            self.flushes.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_compactions_total",
            "Total number of compactions",
            self.compactions.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_set_latency_us_total",
            "Total microseconds spent in set operations",
            self.set_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_get_latency_us_total",
            "Total microseconds spent in get operations",
            self.get_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_delete_latency_us_total",
            "Total microseconds spent in delete operations",
            self.delete_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_scan_latency_us_total",
            "Total microseconds spent in scan operations",
            self.scan_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_flush_latency_us_total",
            "Total microseconds spent in flush operations",
            self.flush_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_compaction_latency_us_total",
            "Total microseconds spent in compaction operations",
            self.compaction_latency_us.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_cache_hits_total",
            "Total number of cache hits",
            self.cache_hits.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_cache_misses_total",
            "Total number of cache misses",
            self.cache_misses.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_bloom_filter_negatives_total",
            "Total number of bloom filter negatives (keys definitively not present)",
            self.bloom_filter_negatives.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_errors_total",
            "Total number of errors",
            self.errors.load(Ordering::Relaxed)
        );

        // Write amplification metrics
        prom_counter!(
            "apexstore_sstable_bytes_written_total",
            "Total bytes written to SSTables via flush and compaction",
            self.total_sstable_bytes_written.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_wal_bytes_written_total",
            "Total bytes written to WAL",
            self.total_wal_bytes_written.load(Ordering::Relaxed)
        );
        prom_counter!(
            "apexstore_sstable_bytes_read_total",
            "Total bytes read from SSTables",
            self.total_sstable_bytes_read.load(Ordering::Relaxed)
        );

        // Read amplification gauge
        out.push_str("# HELP apexstore_read_amplification Read amplification factor (SST bytes read per user operation)\n");
        out.push_str("# TYPE apexstore_read_amplification gauge\n");
        out.push_str("apexstore_read_amplification ");
        // Format f64 without unnecessary trailing zeros
        let ra = self.read_amplification();
        out.push_str(&format!("{:.6}", ra));
        out.push('\n');

        out
    }
}

/// Helper: format a `u64` without allocating a temporary `String`.
/// Falls back to generic formatting when `itoa` is not available.
fn itoa_dispatch(n: u64) -> String {
    // Use std formatting — itoa is not a dependency, and std fmt is fine
    // for the Prometheus endpoint which is not in the hot path.
    n.to_string()
}

impl From<&EngineMetrics> for MetricsSnapshot {
    fn from(m: &EngineMetrics) -> Self {
        m.snapshot()
    }
}

impl From<Arc<EngineMetrics>> for MetricsSnapshot {
    fn from(m: Arc<EngineMetrics>) -> Self {
        m.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_default_zero() {
        let m = EngineMetrics::new();
        let snap = m.snapshot();
        assert_eq!(snap.sets, 0);
        assert_eq!(snap.gets, 0);
        assert_eq!(snap.errors, 0);
    }

    #[test]
    fn test_record_set() {
        let m = EngineMetrics::new();
        m.record_set(42);
        let snap = m.snapshot();
        assert_eq!(snap.sets, 1);
        assert_eq!(snap.set_latency_us, 42);
    }

    #[test]
    fn test_record_get() {
        let m = EngineMetrics::new();
        m.record_get(10);
        m.record_get(20);
        let snap = m.snapshot();
        assert_eq!(snap.gets, 2);
        assert_eq!(snap.get_latency_us, 30);
    }

    #[test]
    fn test_record_delete() {
        let m = EngineMetrics::new();
        m.record_delete(5);
        let snap = m.snapshot();
        assert_eq!(snap.deletes, 1);
        assert_eq!(snap.delete_latency_us, 5);
    }

    #[test]
    fn test_record_scan() {
        let m = EngineMetrics::new();
        m.record_scan(100);
        let snap = m.snapshot();
        assert_eq!(snap.scans, 1);
        assert_eq!(snap.scan_latency_us, 100);
    }

    #[test]
    fn test_record_batch() {
        let m = EngineMetrics::new();
        m.record_batch_sets(5);
        m.record_batch_deletes(3);
        let snap = m.snapshot();
        assert_eq!(snap.batch_sets, 5);
        assert_eq!(snap.batch_deletes, 3);
    }

    #[test]
    fn test_record_flush() {
        let m = EngineMetrics::new();
        m.record_flush(200);
        let snap = m.snapshot();
        assert_eq!(snap.flushes, 1);
        assert_eq!(snap.flush_latency_us, 200);
    }

    #[test]
    fn test_record_compaction() {
        let m = EngineMetrics::new();
        m.record_compaction(1500);
        let snap = m.snapshot();
        assert_eq!(snap.compactions, 1);
        assert_eq!(snap.compaction_latency_us, 1500);
    }

    #[test]
    fn test_record_cache() {
        let m = EngineMetrics::new();
        m.record_cache_hit();
        m.record_cache_miss();
        m.record_bloom_negative();
        let snap = m.snapshot();
        assert_eq!(snap.cache_hits, 1);
        assert_eq!(snap.cache_misses, 1);
        assert_eq!(snap.bloom_filter_negatives, 1);
    }

    #[test]
    fn test_record_error() {
        let m = EngineMetrics::new();
        m.record_error();
        m.record_error();
        let snap = m.snapshot();
        assert_eq!(snap.errors, 2);
    }

    #[test]
    fn test_format_prometheus() {
        let m = EngineMetrics::new();
        m.record_set(42);
        m.record_get(10);
        m.record_delete(5);
        m.record_scan(100);
        m.record_error();

        let output = m.format_prometheus();

        // Should contain HELP lines
        assert!(output.contains("# HELP apexstore_sets_total"));
        assert!(output.contains("# TYPE apexstore_sets_total counter"));
        assert!(output.contains("apexstore_sets_total 1"));
        assert!(output.contains("apexstore_errors_total 1"));

        // Latency accumulators
        assert!(output.contains("apexstore_set_latency_us_total 42"));
        assert!(output.contains("apexstore_get_latency_us_total 10"));

        // Write amplification metrics
        assert!(output.contains("# HELP apexstore_sstable_bytes_written_total"));
        assert!(output.contains("# HELP apexstore_wal_bytes_written_total"));
        assert!(output.contains("# HELP apexstore_sstable_bytes_read_total"));

        // Read amplification gauge
        assert!(output.contains("# HELP apexstore_read_amplification"));
        assert!(output.contains("# TYPE apexstore_read_amplification gauge"));

        // Each metric has HELP + TYPE + value (3 lines), plus some extra
        assert!(!output.is_empty());
    }

    #[test]
    fn test_write_amplification() {
        let m = EngineMetrics::new();
        // No reads yet — should give 0 / 1 = 0.0
        assert_eq!(m.write_amplification(), 0.0);

        // Record some writes
        m.record_sstable_write(1000);
        m.record_wal_write(500);
        // Still no reads — write_amp = 1500 / 1 = 1500
        assert_eq!(m.write_amplification(), 1500.0);

        // Record reads
        m.record_sstable_read(2000);
        // write_amp = 1500 / 2000 = 0.75
        assert!((m.write_amplification() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn test_read_amplification() {
        let m = EngineMetrics::new();
        // No user ops yet — denominator is max(1) = 1, no bytes read → 0.0
        assert_eq!(m.read_amplification(), 0.0);

        // Record some SSTable reads
        m.record_sstable_read(1000);
        // Still no user ops — read_amp = 1000 / 1 = 1000
        assert_eq!(m.read_amplification(), 1000.0);

        // Record user operations
        m.record_get(10);
        m.record_scan(20);
        // read_amp = 1000 / 2 = 500
        assert!((m.read_amplification() - 500.0).abs() < f64::EPSILON);

        // More reads
        m.record_sstable_read(500);
        // read_amp = 1500 / 2 = 750
        assert!((m.read_amplification() - 750.0).abs() < f64::EPSILON);
    }
}
