use apexstore::infra::config::LsmConfig;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::Rng;
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
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', remaining.min(64)));
    value.truncate(value_size);
    value
}

/// Benchmark YCSB Type A: 50% read, 50% write (uniform)
fn bench_ycsb_type_a(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000]
    } else {
        vec![10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("ycsb_type_a");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_a");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(nk * 220)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        if rng.gen::<f32>() < 0.5 {
                            let index = rng.gen_range(0..nk);
                            let _ = engine.get(&keys[index]).unwrap();
                        } else {
                            let index = rng.gen_range(0..nk);
                            let new_value = generate_value(rng.gen_range(0..10_000), 100);
                            engine.set(keys[index].clone(), new_value).unwrap();
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

/// Benchmark YCSB Type B: 95% read, 5% write (read-heavy)
fn bench_ycsb_type_b(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000]
    } else {
        vec![10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("ycsb_type_b");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_b");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(16 * 1024 * 1024)
                        .block_cache_size_mb(512)
                        .bloom_false_positive_rate(0.001)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        if rng.gen::<f32>() < 0.95 {
                            let index = rng.gen_range(0..nk);
                            let _ = engine.get(&keys[index]).unwrap();
                        } else {
                            let index = rng.gen_range(0..nk);
                            let new_value = generate_value(rng.gen_range(0..10_000), 100);
                            engine.set(keys[index].clone(), new_value).unwrap();
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

/// Benchmark YCSB Type C: 100% read (read-only)
fn bench_ycsb_type_c(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000, 100_000]
    } else {
        vec![10_000, 100_000, 1_000_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("ycsb_type_c");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_c");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(16 * 1024 * 1024)
                        .block_cache_size_mb(512)
                        .bloom_false_positive_rate(0.001)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                let warmup = nk.min(10_000);
                for key in keys.iter().take(warmup) {
                    let _ = engine.get(key.as_str()).unwrap();
                }

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        let index = rng.gen_range(0..nk);
                        let _ = engine.get(&keys[index]).unwrap();
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark balanced workload: 50% read, 50% write
fn bench_workload_balanced(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000]
    } else {
        vec![10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("workload_balanced");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_balanced");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(32 * 1024 * 1024)
                        .block_cache_size_mb(256)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        if rng.gen::<f32>() < 0.5 {
                            let index = rng.gen_range(0..nk);
                            let _ = engine.get(&keys[index]).unwrap();
                        } else {
                            let index = rng.gen_range(0..nk);
                            let new_value = generate_value(rng.gen_range(0..10_000), 100);
                            engine.set(keys[index].clone(), new_value).unwrap();
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

/// Benchmark read-heavy workload
fn bench_workload_read_heavy(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000]
    } else {
        vec![10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("workload_read_heavy");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_read_heavy");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(16 * 1024 * 1024)
                        .block_cache_size_mb(512)
                        .bloom_false_positive_rate(0.001)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();

                for (i, key) in keys.iter().enumerate() {
                    let value = generate_value(i, 100);
                    engine.set(key.clone(), value).unwrap();
                }

                engine.flush_memtable().unwrap();

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        if rng.gen::<f32>() < 0.9 {
                            let index = rng.gen_range(0..nk);
                            let _ = engine.get(&keys[index]).unwrap();
                        } else {
                            let index = rng.gen_range(0..nk);
                            let new_value = generate_value(rng.gen_range(0..10_000), 100);
                            engine.set(keys[index].clone(), new_value).unwrap();
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

/// Benchmark write-heavy workload
fn bench_workload_write_heavy(c: &mut Criterion) {
    let num_keys_arr: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10_000]
    } else {
        vec![10_000, 100_000]
    };
    for num_keys in num_keys_arr {
        let mut group = c.benchmark_group("workload_write_heavy");
        group.throughput(Throughput::Bytes(100));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_write_heavy");
                let mut engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(32 * 1024 * 1024)
                        .block_cache_size_mb(128)
                        .build()
                        .unwrap(),
                    std::sync::Arc::new(apexstore::storage::cache::GlobalBlockCache::new(
                        100, 4096,
                    )),
                )
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();

                for (i, key) in keys.iter().enumerate() {
                    let value = generate_value(i, 100);
                    engine.set(key.clone(), value).unwrap();
                }

                let mut rng = rand::thread_rng();

                b.iter(|| {
                    for _ in 0..1000 {
                        if rng.gen::<f32>() < 0.1 {
                            let index = rng.gen_range(0..nk);
                            let _ = engine.get(&keys[index]).unwrap();
                        } else {
                            let index = rng.gen_range(0..nk);
                            let new_value = generate_value(rng.gen_range(0..10_000), 100);
                            engine.set(keys[index].clone(), new_value).unwrap();
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

criterion_group!(
    name = mixed_benches;
    config = configure_criterion();
    targets = bench_ycsb_type_a, bench_ycsb_type_b, bench_ycsb_type_c, bench_workload_balanced, bench_workload_read_heavy, bench_workload_write_heavy
);

criterion_main!(mixed_benches);
