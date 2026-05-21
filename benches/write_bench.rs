use apexstore::infra::config::LsmConfig;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::PathBuf;
use tempfile::TempDir;

fn configure_criterion() -> Criterion {
    let mut c = Criterion::default();
    if std::env::var("CI").is_ok() {
        c = c
            .sample_size(5)
            .warm_up_time(std::time::Duration::from_millis(500))
            .measurement_time(std::time::Duration::from_secs(1));
    }
    c
}

/// Setup a temporary directory for benchmark testing
fn setup_temp_dir(name: &str) -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join(name);
    (temp_dir, path)
}

/// Generate deterministic test key
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

/// Generate deterministic test value
fn generate_value(index: usize, value_size: usize) -> Vec<u8> {
    let pattern = format!("val_{}_{:08x}_", index, index);
    let remaining = value_size.saturating_sub(pattern.len());
    let fill_count = remaining.min(64);
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', fill_count));
    value.truncate(value_size);
    value
}

/// Benchmark single write operations with varying key/value sizes
fn bench_single_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_single");

    let value_sizes: &[usize] = if std::env::var("CI").is_ok() {
        &[10, 1024]
    } else {
        &[10, 100, 1024, 10240]
    };
    for &value_size in value_sizes {
        group.throughput(Throughput::Bytes(value_size as u64));

        group.bench_with_input(BenchmarkId::from_parameter(value_size), &(), |b, &_| {
            let (temp_dir, data_dir) = setup_temp_dir("single_write");
            let mut engine = apexstore::LsmEngine::new_from_config(
                &LsmConfig::builder()
                    .dir_path(data_dir.clone())
                    .memtable_max_size(16 * 1024 * 1024)
                    .build()
                    .unwrap(),
                apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            let key = String::from("benchmark_key");
            let value = vec![b'x'; value_size];

            b.iter(|| {
                engine.set(key.clone(), value.clone()).unwrap();
            });

            drop(engine);
            drop(temp_dir);
        });
    }

    group.finish();
}

/// Benchmark batch write operations
fn bench_batch_write(c: &mut Criterion) {
    let batch_sizes: &[usize] = if std::env::var("CI").is_ok() {
        &[1_000, 10_000]
    } else {
        &[1_000, 10_000, 100_000]
    };
    for &batch_size in batch_sizes {
        let mut group = c.benchmark_group(format!("write_batch_{}", batch_size));
        let total_bytes = batch_size * 110;
        group.throughput(Throughput::Bytes(total_bytes as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &bs| {
                let (temp_dir, data_dir) = setup_temp_dir("batch_write");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(bs * 220)
                        .build()
                        .unwrap(),
                    apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
                )
                .unwrap();

                let keys: Vec<String> = (0..bs).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..bs).map(|i| generate_value(i, 100)).collect();

                b.iter(|| {
                    for (key, value) in keys.iter().zip(values.iter()) {
                        engine.set(key.clone(), value.clone()).unwrap();
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark memtable flush performance
fn bench_memtable_flush(c: &mut Criterion) {
    let memtable_sizes: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![8 * 1024 * 1024]
    } else {
        vec![8 * 1024 * 1024, 16 * 1024 * 1024, 32 * 1024 * 1024]
    };
    for memtable_size in memtable_sizes {
        let mut group =
            c.benchmark_group(format!("memtable_flush_{}", memtable_size / 1024 / 1024));
        group.throughput(Throughput::Bytes(memtable_size as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(memtable_size / 1024 / 1024),
            &memtable_size,
            |b, &ms| {
                let (temp_dir, data_dir) = setup_temp_dir("memtable_flush");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(ms)
                        .build()
                        .unwrap(),
                    apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
                )
                .unwrap();

                let records_per_batch = (ms / 2) / 110;
                let keys: Vec<String> = (0..records_per_batch)
                    .map(|i| generate_key(i, 10))
                    .collect();
                let values: Vec<Vec<u8>> = (0..records_per_batch)
                    .map(|i| generate_value(i, 100))
                    .collect();

                b.iter(|| {
                    for (key, value) in keys.iter().zip(values.iter()) {
                        engine.set(key.clone(), value.clone()).unwrap();
                    }
                    engine.flush_memtable().unwrap();
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark SSTable write performance during flush
fn bench_sstable_flush(c: &mut Criterion) {
    let records = if std::env::var("CI").is_ok() {
        10_000usize
    } else {
        100_000usize
    };
    let mut group = c.benchmark_group("sstable_flush");
    let flush_mb = (records * 110) as f64 / 1_000_000.0;
    group.throughput(Throughput::Bytes((records * 110) as u64));

    group.bench_with_input(BenchmarkId::from_parameter(records), &records, |b, &_r| {
        let (temp_dir, data_dir) = setup_temp_dir("sstable_flush");
        let mut engine = apexstore::LsmEngine::new_from_config(
            &LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(10 * 1024 * 1024)
                .build()
                .unwrap(),
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        let records = _r;
        let keys: Vec<String> = (0..records).map(|i| generate_key(i, 10)).collect();
        let values: Vec<Vec<u8>> = (0..records).map(|i| generate_value(i, 100)).collect();

        b.iter(|| {
            for (key, value) in keys.iter().zip(values.iter()) {
                engine.set(key.clone(), value.clone()).unwrap();
            }
        });

        let flush_start = std::time::Instant::now();
        engine.flush_memtable().unwrap();
        let flush_duration = flush_start.elapsed().as_secs_f64();

        println!("Flush time: {:.3}s for ~{:.1}MB", flush_duration, flush_mb);
        println!("Throughput: {:.2} MB/s", flush_mb / flush_duration);

        drop(engine);
        drop(temp_dir);
    });

    group.finish();
}

/// Benchmark write operations across different data sizes
fn bench_write_by_size(c: &mut Criterion) {
    let configs: &[(usize, usize)] = if std::env::var("CI").is_ok() {
        &[(10, 100), (100, 10000)]
    } else {
        &[
            (10usize, 10),
            (10, 100),
            (100, 100),
            (100, 1000),
            (100, 10000),
        ]
    };

    for (key_size, value_size) in configs {
        let mut group = c.benchmark_group(format!("write_size_{}_{}", key_size, value_size));
        group.throughput(Throughput::Bytes((key_size + value_size) as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", key_size, value_size)),
            &(),
            |b, &_| {
                let (temp_dir, data_dir) = setup_temp_dir("write_by_size");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(16 * 1024 * 1024)
                        .build()
                        .unwrap(),
                    apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
                )
                .unwrap();

                let key = generate_key(0, *key_size);
                let value = generate_value(0, *value_size);

                b.iter(|| {
                    engine.set(key.clone(), value.clone()).unwrap();
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

criterion_group!(
    name = write_benches;
    config = configure_criterion();
    targets = bench_single_write, bench_batch_write, bench_memtable_flush, bench_sstable_flush, bench_write_by_size
);

criterion_main!(write_benches);
