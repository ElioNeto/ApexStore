use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use criterion::{criterion_group, criterion_main, Criterion};
use std::path::PathBuf;
use std::time::Instant;
use tempfile::TempDir;

fn configure_criterion() -> Criterion {
    let mut c = Criterion::default();
    if std::env::var("CI").is_ok() {
        c = c
            .sample_size(10)
            .warm_up_time(std::time::Duration::from_secs(1))
            .measurement_time(std::time::Duration::from_secs(3));
    }
    c
}

fn setup_temp_dir(name: &str) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join(name);
    (temp_dir, path)
}

fn generate_key(index: usize, key_size: usize) -> String {
    let prefix = format!("key_{}_{:08x}_", index, index);
    let padding_size = key_size.saturating_sub(prefix.len());
    let padding = if padding_size > 0 {
        &"x".repeat(padding_size.min(64))
    } else {
        ""
    };
    format!("{}{}", prefix, padding)
}

fn generate_value(index: usize, value_size: usize) -> Vec<u8> {
    let pattern = format!("val_{}_{:08x}_", index, index);
    let remaining = value_size.saturating_sub(pattern.len());
    let fill_count = remaining.min(64);
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', fill_count));
    value.truncate(value_size);
    value
}

/// Compute P50, P95, P99 from a sorted slice of latencies (in ns).
fn compute_percentiles(latencies_ns: &mut Vec<u64>) -> (f64, f64, f64) {
    if latencies_ns.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    latencies_ns.sort_unstable();
    let len = latencies_ns.len();
    let p50 = latencies_ns[(len as f64 * 0.50) as usize.min(len - 1)] as f64 / 1000.0;
    let p95 = latencies_ns[(len as f64 * 0.95) as usize.min(len - 1)] as f64 / 1000.0;
    let p99 = latencies_ns[(len as f64 * 0.99) as usize.min(len - 1)] as f64 / 1000.0;
    (p50, p95, p99)
}

/// Helper to build a cheap LsmConfig for memtable-only benchmarks
fn memtable_config(data_dir: PathBuf, num_keys: usize) -> LsmConfig {
    LsmConfig::builder()
        .dir_path(data_dir)
        .memtable_max_size((num_keys as f64 * 220.0) as usize)
        .build()
        .unwrap()
}

/// Helper to build an LsmConfig that forces flushes (for SSTable benchmarks)
fn sstable_config(data_dir: PathBuf, num_keys: usize) -> LsmConfig {
    LsmConfig::builder()
        .dir_path(data_dir)
        .memtable_max_size((num_keys as f64 * 110.0 / 2.0) as usize)
        .block_cache_size_mb(1)
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Read latency benchmarks — memtable (all data fits in memory)
// ---------------------------------------------------------------------------

fn bench_read_latency_memtable_1k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_memtable_1k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_mem_1k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &memtable_config(data_dir, 1_000),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }

    let mut all_latencies = Vec::with_capacity(1_000);
    group.bench_function("memtable_1k", |b| {
        b.iter(|| {
            all_latencies.clear();
            for key in keys.iter() {
                let start = Instant::now();
                let _ = engine.get(key.as_str()).unwrap();
                all_latencies.push(start.elapsed().as_nanos() as u64);
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (memtable, 1k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

fn bench_read_latency_memtable_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_memtable_100k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_mem_100k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &memtable_config(data_dir, 100_000),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..100_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..100_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }

    let mut all_latencies = Vec::with_capacity(1_000);
    group.bench_function("memtable_100k", |b| {
        b.iter(|| {
            all_latencies.clear();
            for key in keys.iter() {
                let start = Instant::now();
                let _ = engine.get(key.as_str()).unwrap();
                all_latencies.push(start.elapsed().as_nanos() as u64);
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (memtable, 100k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

// ---------------------------------------------------------------------------
// Read latency benchmarks — SSTable (data flushed to disk)
// ---------------------------------------------------------------------------

fn bench_read_latency_sstable_1k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_sstable_1k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_sst_1k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &sstable_config(data_dir, 1_000),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }

    let mut all_latencies = Vec::with_capacity(1_000);
    group.bench_function("sstable_cold_1k", |b| {
        b.iter(|| {
            all_latencies.clear();
            for key in keys.iter() {
                let start = Instant::now();
                let _ = engine.get(key.as_str()).unwrap();
                all_latencies.push(start.elapsed().as_nanos() as u64);
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (sstable, 1k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

fn bench_read_latency_sstable_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_sstable_100k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_sst_100k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &sstable_config(data_dir, 100_000),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..100_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..100_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }

    let mut all_latencies = Vec::with_capacity(1_000);
    group.bench_function("sstable_cold_100k", |b| {
        b.iter(|| {
            all_latencies.clear();
            for key in keys.iter() {
                let start = Instant::now();
                let _ = engine.get(key.as_str()).unwrap();
                all_latencies.push(start.elapsed().as_nanos() as u64);
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (sstable, 100k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

// ---------------------------------------------------------------------------
// Write latency benchmarks — 1k keys (single memtable)
// ---------------------------------------------------------------------------

fn bench_write_latency_1k(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_latency_1k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_write_1k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &memtable_config(data_dir, 1_000),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();

    let mut all_latencies = Vec::with_capacity(keys.len());
    group.bench_function("write_1k", |b| {
        b.iter(|| {
            all_latencies.clear();
            // Re-create keys/values each iteration to avoid key conflicts
            let iteration_keys: Vec<String> = (0..1_000).map(|i| generate_key(i + 999_999, 10)).collect();
            let iteration_values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i + 999_999, 100)).collect();
            for (k, v) in iteration_keys.iter().zip(iteration_values.iter()) {
                let start = Instant::now();
                engine.set(k.clone(), v.clone()).unwrap();
                all_latencies.push(start.elapsed().as_nanos() as u64);
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (write, 1k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

// ---------------------------------------------------------------------------
// Write latency benchmarks — 100k keys (triggers flushes)
// ---------------------------------------------------------------------------

fn bench_write_latency_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_latency_100k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_write_100k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &LsmConfig::builder()
            .dir_path(data_dir)
            .memtable_max_size(4 * 1024 * 1024) // 4MB — will trigger multiple flushes
            .build()
            .unwrap(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    let sample_size = 1000; // track latency for 1k sampled writes
    let mut all_latencies = Vec::with_capacity(sample_size);
    group.bench_function("write_100k", |b| {
        b.iter(|| {
            all_latencies.clear();
            for i in 0..100_000 {
                let key = generate_key(i, 10);
                let value = generate_value(i, 100);
                let start = Instant::now();
                engine.set(key, value).unwrap();
                if i % (100_000 / sample_size) == 0 {
                    all_latencies.push(start.elapsed().as_nanos() as u64);
                }
            }
        });
    });
    let (p50, p95, p99) = compute_percentiles(&mut all_latencies);
    println!(
        "  → P50={p50:.1}µs  P95={p95:.1}µs  P99={p99:.1}µs  (write, 100k keys)"
    );
    group.finish();
    drop(engine);
    drop(temp_dir);
}

criterion_group!(
    name = latency_benches;
    config = configure_criterion();
    targets = bench_read_latency_memtable_1k, bench_read_latency_memtable_100k, bench_read_latency_sstable_1k, bench_read_latency_sstable_100k, bench_write_latency_1k, bench_write_latency_100k
);
criterion_main!(latency_benches);
