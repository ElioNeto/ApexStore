use apexstore::infra::config::{CoreConfig, LsmConfig, StorageConfig};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::Rng;
use std::path::PathBuf;
use tempfile::TempDir;

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
    for num_keys in [10_000usize, 100_000] {
        let mut group = c.benchmark_group("ycsb_type_a");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_a");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: nk * 220,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 64,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.01,
                    },
                })
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
    for num_keys in [10_000usize, 100_000] {
        let mut group = c.benchmark_group("ycsb_type_b");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_b");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: 16 * 1024 * 1024,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 512,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.001,
                    },
                })
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
    for num_keys in [10_000usize, 100_000, 1_000_000] {
        let mut group = c.benchmark_group("ycsb_type_c");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("ycsb_type_c");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: 16 * 1024 * 1024,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 1024,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.001,
                    },
                })
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> = (0..nk).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                engine.flush_memtable().unwrap();

                // Warm cache
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
    for num_keys in [10_000usize, 100_000] {
        let mut group = c.benchmark_group("workload_balanced");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_balanced");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: 32 * 1024 * 1024,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 256,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.01,
                    },
                })
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
    for num_keys in [10_000usize, 100_000] {
        let mut group = c.benchmark_group("workload_read_heavy");
        group.throughput(Throughput::Elements(1000));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_read_heavy");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: 16 * 1024 * 1024,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 512,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.001,
                    },
                })
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();

                for i in 0..nk {
                    let key = keys[i].clone();
                    let value = generate_value(i, 100);
                    engine.set(key, value).unwrap();
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
    for num_keys in [10_000usize, 100_000] {
        let mut group = c.benchmark_group("workload_write_heavy");
        group.throughput(Throughput::Bytes(100));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("workload_write_heavy");
                let engine = apexstore::LsmEngine::new(LsmConfig {
                    core: CoreConfig {
                        dir_path: data_dir.clone(),
                        memtable_max_size: 32 * 1024 * 1024,
                    },
                    storage: StorageConfig {
                        block_size: 4096,
                        block_cache_size_mb: 128,
                        sparse_index_interval: 16,
                        bloom_false_positive_rate: 0.01,
                    },
                })
                .unwrap();

                let keys: Vec<String> = (0..nk).map(|i| generate_key(i, 10)).collect();

                for i in 0..nk {
                    let key = keys[i].clone();
                    let value = generate_value(i, 100);
                    engine.set(key, value).unwrap();
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
    mixed_benches,
    bench_ycsb_type_a,
    bench_ycsb_type_b,
    bench_ycsb_type_c,
    bench_workload_balanced,
    bench_workload_read_heavy,
    bench_workload_write_heavy,
);

criterion_main!(mixed_benches);
