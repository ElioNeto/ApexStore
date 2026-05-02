use apexstore::infra::config::LsmConfig;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::path::PathBuf;
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

/// Benchmark read operations with all keys in MemTable
fn bench_read_memtable(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![1_000, 10_000]
    } else {
        vec![1_000, 10_000, 100_000, 1_000_000]
    };
    for &num_keys in &num_keys_arr {
        let mut group = c.benchmark_group("read_memtable");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("read_memtable");
                let mut engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * 220)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                let benchmark_keys: Vec<String> = keys.iter().step_by(nk / 1000).cloned().collect();
                let benchmark_values: Vec<Vec<u8>> =
                    values.iter().step_by(nk / 1000).cloned().collect();

                b.iter(|| {
                    for (key, value) in benchmark_keys.iter().zip(benchmark_values.iter()) {
                        let result = engine.get(key.as_str()).unwrap();
                        assert_eq!(result, Some(value.clone()));
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark read operations with all keys in SSTable (cold cache)
fn bench_read_sstable_cold(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![1_000, 10_000]
    } else {
        vec![1_000, 10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("read_sstable_cold");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("read_sstable_cold");
                let mut engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * 110 / 2)
                        .block_cache_size_mb(1)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                let benchmark_keys: Vec<String> = keys.iter().step_by(nk / 1000).cloned().collect();

                b.iter(|| {
                    for key in benchmark_keys.iter() {
                        let result = engine.get(key.as_str()).unwrap();
                        assert!(result.is_some());
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark read operations with cache warmed up
fn bench_read_sstable_warm(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![1_000, 10_000]
    } else {
        vec![1_000, 10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("read_sstable_warm");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("read_sstable_warm");
                let mut engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * 110 / 2)
                        .block_cache_size_mb(256)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                // Warm the cache
                for key in &keys {
                    let _ = engine.get(key.as_str()).unwrap();
                }

                let benchmark_keys: Vec<String> = keys.iter().step_by(nk / 1000).cloned().collect();

                b.iter(|| {
                    for key in benchmark_keys.iter() {
                        let result = engine.get(key.as_str()).unwrap();
                        assert!(result.is_some());
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark Bloom filter effectiveness
fn bench_bloom_filter(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000, 100_000]
    } else {
        vec![10_000, 100_000, 1_000_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("bloom_filter");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("bloom_filter");
                let mut engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * 110 / 2)
                        .block_cache_size_mb(1)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let existing_keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in existing_keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                let non_existing_keys: Vec<String> =
                    (nk..nk * 2).map(|i| generate_key(i, 10)).collect();

                b.iter(|| {
                    for key in non_existing_keys.iter() {
                        let result = engine.get(key.as_str()).unwrap();
                        if result.is_some() {
                            // This would be a false positive
                        }
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark read with various cache configurations
fn bench_read_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency");

    group.bench_function("memtable_1k", |b| {
        let (temp_dir, data_dir) = setup_temp_dir("read_latency_memtable");
        let mut engine = apexstore::LsmEngine::new(
            LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(1_000 * 220)
                .build()
                .unwrap(),
        )
        .unwrap();

        let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
        let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();

        for (key, value) in keys.iter().zip(values.iter()) {
            engine.set(key.clone(), value.clone()).unwrap();
        }

        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });

        drop(engine);
        drop(temp_dir);
    });

    group.bench_function("sstable_cold_1k", |b| {
        let (temp_dir, data_dir) = setup_temp_dir("read_latency_sstable");
        let mut engine = apexstore::LsmEngine::new(
            LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(1_000 * 110 / 2)
                .block_cache_size_mb(1)
                .build()
                .unwrap(),
        )
        .unwrap();

        let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
        let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();

        for (key, value) in keys.iter().zip(values.iter()) {
            engine.set(key.clone(), value.clone()).unwrap();
        }

        engine.flush_memtable().unwrap();

        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });

        drop(engine);
        drop(temp_dir);
    });

    group.finish();
}

/// Benchmark sequential scan performance
fn bench_scan_sequential(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![1_000, 10_000]
    } else {
        vec![1_000, 10_000, 100_000]
    };
    for &num_keys in &num_keys_arr {
        let mut group = c.benchmark_group("scan_sequential");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &(num_keys, 10, 100),
            |b, &(nk, ks, vs)| {
                let (temp_dir, data_dir) = setup_temp_dir("scan_sequential");
                let mut engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * (ks + vs) / 2)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, ks)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, vs)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                b.iter(|| {
                    let results = engine.scan_cf("default", None, None, None).unwrap();
                    assert_eq!(results.len(), nk);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

criterion_group!(
    name = read_benches;
    config = configure_criterion();
    targets = bench_read_memtable, bench_read_sstable_cold, bench_read_sstable_warm, bench_bloom_filter, bench_read_latency, bench_scan_sequential
);

criterion_main!(read_benches);
