use apexstore::core::engine::LsmEngine;
use apexstore::infra::config::LsmConfig;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tempfile::TempDir;

/// Benchmark-specific configuration
#[derive(Debug, Clone, Copy)]
pub struct BenchmarkConfig {
    pub key_size: usize,
    pub value_size: usize,
    pub num_keys: usize,
    pub memtable_max_size: usize,
    pub block_cache_size_mb: usize,
    pub bloom_false_positive_rate: f64,
    pub sparse_index_interval: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            key_size: 10,
            value_size: 100,
            num_keys: 100_000,
            memtable_max_size: 16 * 1024 * 1024, // 16MB
            block_cache_size_mb: 256,
            bloom_false_positive_rate: 0.01,
            sparse_index_interval: 16,
        }
    }
}

impl BenchmarkConfig {
    pub fn with_key_size(mut self, size: usize) -> Self {
        self.key_size = size;
        self
    }

    pub fn with_value_size(mut self, size: usize) -> Self {
        self.value_size = size;
        self
    }

    pub fn with_num_keys(mut self, count: usize) -> Self {
        self.num_keys = count;
        self
    }

    pub fn write_heavy() -> Self {
        Self {
            key_size: 10,
            value_size: 100,
            num_keys: 1_000_000,
            memtable_max_size: 16 * 1024 * 1024,
            block_cache_size_mb: 256,
            ..Default::default()
        }
    }

    pub fn read_heavy() -> Self {
        Self {
            key_size: 10,
            value_size: 100,
            num_keys: 1_000_000,
            memtable_max_size: 8 * 1024 * 1024,
            block_cache_size_mb: 1024,
            bloom_false_positive_rate: 0.001,
            ..Default::default()
        }
    }

    pub fn balanced() -> Self {
        Self {
            key_size: 10,
            value_size: 100,
            num_keys: 1_000_000,
            memtable_max_size: 16 * 1024 * 1024,
            block_cache_size_mb: 256,
            ..Default::default()
        }
    }
}

/// Setup a temporary directory for benchmark testing
pub fn setup_temp_dir(name: &str) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join(name);
    (temp_dir, path)
}

/// Create an LsmEngine with benchmark-specific configuration
pub fn create_bench_engine(config: &BenchmarkConfig, data_dir: &std::path::Path) -> LsmEngine {
    let lsm_config = LsmConfig::builder()
        .dir_path(data_dir.to_path_buf())
        .memtable_max_size(config.memtable_max_size)
        .block_cache_size_mb(config.block_cache_size_mb)
        .bloom_false_positive_rate(config.bloom_false_positive_rate)
        .sparse_index_interval(config.sparse_index_interval)
        .build()
        .expect("Invalid LsmConfig");

    LsmEngine::new(lsm_config).expect("Failed to create LsmEngine")
}

/// Generate deterministic test data
pub fn generate_key(index: usize, key_size: usize) -> String {
    let prefix = format!("key_{}_{:08x}_", index, index);
    let padding_size = key_size.saturating_sub(prefix.len());
    let padding = if padding_size > 0 {
        &"x".repeat(padding_size.min(64))
    } else {
        ""
    };
    format!("{}{}", prefix, padding)
}

/// Generate deterministic test value
pub fn generate_value(index: usize, value_size: usize) -> Vec<u8> {
    let pattern = format!("val_{}_{:08x}_", index, index);
    let remaining = value_size.saturating_sub(pattern.len());
    let fill_count = remaining.min(64);
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', fill_count));
    value.truncate(value_size);
    value
}

/// Generate log records for testing
pub fn create_test_records(
    count: usize,
    key_size: usize,
    value_size: usize,
) -> Vec<apexstore::core::log_record::LogRecord> {
    (0..count)
        .map(|i| {
            let key = generate_key(i, key_size);
            let value = generate_value(i, value_size);
            apexstore::core::log_record::LogRecord::new(key, value)
        })
        .collect()
}

/// Pre-populate engine with keys for read benchmarks
pub fn pre_populate_for_reads(
    engine: &mut LsmEngine,
    num_keys: usize,
    key_size: usize,
    value_size: usize,
) -> (Vec<String>, Vec<Vec<u8>>) {
    let keys: Vec<String> = (0..num_keys).map(|i| generate_key(i, key_size)).collect();

    let values: Vec<Vec<u8>> = (0..num_keys)
        .map(|i| generate_value(i, value_size))
        .collect();

    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }

    (keys, values)
}

/// Metrics for benchmark operations
#[derive(Debug, Default)]
pub struct Metrics {
    pub ops_count: AtomicUsize,
    pub total_latency_ns: AtomicUsize,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_op(&self, latency_ns: u64) {
        self.ops_count.fetch_add(1, Ordering::Relaxed);
        self.total_latency_ns
            .fetch_add(latency_ns as usize, Ordering::Relaxed);
    }

    pub fn throughput(&self) -> f64 {
        self.ops_count.load(Ordering::Relaxed) as f64
    }

    pub fn avg_latency_ns(&self) -> f64 {
        let count = self.ops_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        self.total_latency_ns.load(Ordering::Relaxed) as f64 / count as f64
    }

    pub fn avg_latency_us(&self) -> f64 {
        self.avg_latency_ns() / 1000.0
    }
}

/// Benchmark iteration/scanning metrics
pub struct ScanMetrics {
    pub keys_scanned: usize,
    pub latency_ns: u64,
    pub throughput_keys_per_sec: f64,
}

impl ScanMetrics {
    pub fn new(keys_scanned: usize, _start: Instant, latency_ns: u64) -> Self {
        Self {
            keys_scanned,
            latency_ns,
            throughput_keys_per_sec: (keys_scanned as f64) / (latency_ns as f64 / 1_000_000_000.0),
        }
    }
}

/// Generate random keys for stress testing
#[allow(unexpected_cfgs)]
pub fn generate_random_keys(count: usize, key_size: usize) -> Vec<String> {
    let hex_chars = "0123456789abcdef";
    (0..count)
        .map(|_| {
            (0..key_size)
                .map(|_| hex_chars.chars().collect::<Vec<_>>()[rand::random::<usize>() % 16])
                .collect::<String>()
        })
        .collect()
}

/// YCSB-style workload distribution
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum YCSBWorkload {
    TypeA, // 50% read, 50% write (uniform)
    TypeB, // 95% read, 5% write (read-heavy)
    TypeC, // 100% read (read-only)
    TypeD, // 95% read, 5% write (trending hot keys)
    TypeE, // 95% read, 5% write (sliding window)
    TypeF, // 50/50 read/write but 1% of keys represent 99% of ops (zipf)
}

impl YCSBWorkload {
    pub fn operation_type(&self) -> (&'static str, f64, f64) {
        match self {
            YCSBWorkload::TypeA => ("Mixed (50/50)", 50.0, 50.0),
            YCSBWorkload::TypeB => ("Read-heavy (95/5)", 95.0, 5.0),
            YCSBWorkload::TypeC => ("Read-only", 100.0, 0.0),
            YCSBWorkload::TypeD => ("Trending (95/5)", 95.0, 5.0),
            YCSBWorkload::TypeE => ("Sliding Window (95/5)", 95.0, 5.0),
            YCSBWorkload::TypeF => ("Zipf (50/50)", 50.0, 50.0),
        }
    }
}

/// Thread-safe counter for benchmark statistics
#[derive(Debug, Default)]
pub struct BenchmarkStats {
    pub ops_count: AtomicUsize,
    pub bytes_processed: AtomicUsize,
    pub errors: AtomicUsize,
}

impl BenchmarkStats {
    pub fn record_op(&self, bytes: usize) {
        self.ops_count.fetch_add(1, Ordering::Relaxed);
        self.bytes_processed.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_throughput(&self, duration_ns: u64) -> f64 {
        let ops = self.ops_count.load(Ordering::Relaxed);
        let duration_sec = duration_ns as f64 / 1_000_000_000.0;
        ops as f64 / duration_sec
    }

    pub fn get_byte_throughput(&self, duration_ns: u64) -> f64 {
        let bytes = self.bytes_processed.load(Ordering::Relaxed);
        let duration_sec = duration_ns as f64 / 1_000_000_000.0;
        bytes as f64 / duration_sec
    }
}

/// Benchmark iteration metrics
pub struct IterationMetrics {
    pub items_processed: usize,
    pub total_time_ns: u64,
    pub throughput_items_per_sec: f64,
}

impl IterationMetrics {
    pub fn from_duration(items: usize, start: Instant) -> Self {
        let elapsed = start.elapsed();
        Self {
            items_processed: items,
            total_time_ns: elapsed.as_nanos() as u64,
            throughput_items_per_sec: items as f64 / elapsed.as_secs_f64(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key() {
        let key = generate_key(123, 100);
        assert_eq!(key.len(), 100);
        assert!(key.starts_with("key_123_0000007b_"));
    }

    #[test]
    fn test_generate_value() {
        let value = generate_value(456, 256);
        assert_eq!(value.len(), 256);
        assert!(value.starts_with(b"val_456_000001c8_"));
    }

    #[test]
    fn test_benchmark_config_default() {
        let config = BenchmarkConfig::default();
        assert_eq!(config.key_size, 10);
        assert_eq!(config.value_size, 100);
        assert_eq!(config.num_keys, 100_000);
    }

    #[test]
    fn test_metrics_recording() {
        let metrics = Metrics::new();
        for i in 0..100 {
            metrics.record_op((i * 10) as u64);
        }
        assert_eq!(metrics.throughput(), 100.0);
        assert_eq!(metrics.avg_latency_us(), 450.0);
    }

    #[test]
    fn test_ycsb_workload() {
        assert_eq!(
            YCSBWorkload::TypeB.operation_type(),
            ("Read-heavy (95/5)", 95.0, 5.0)
        );
        assert_eq!(
            YCSBWorkload::TypeC.operation_type(),
            ("Read-only", 100.0, 0.0)
        );
        assert_eq!(
            YCSBWorkload::TypeA.operation_type(),
            ("Mixed (50/50)", 50.0, 50.0)
        );
    }
}
