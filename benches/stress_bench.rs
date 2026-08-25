use apexstore::infra::config::LsmConfig;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::path::PathBuf;
use tempfile::TempDir;

fn configure_criterion() -> Criterion {
    let mut c = Criterion::default();
    if std::env::var("CI").is_ok() {
        // 10 is Criterion's hard floor: `Criterion::sample_size` asserts
        // `n >= 10` and panics otherwise, which aborted every benchmark before
        // it produced a single measurement whenever CI was set.
        c = c
            .sample_size(10)
            .warm_up_time(std::time::Duration::from_millis(500))
            .measurement_time(std::time::Duration::from_secs(1));
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

/// Benchmark with very large dataset (1M keys)
fn bench_large_dataset_1m(c: &mut Criterion) {
    if std::env::var("CI").is_ok() {
        return; // Skip in CI - too expensive
    }
    let mut group = c.benchmark_group("large_dataset_1m");

    group.bench_with_input(BenchmarkId::from_parameter("1m_keys"), &(), |b, &_| {
        let (temp_dir, data_dir) = setup_temp_dir("large_1m");
        let engine = apexstore::LsmEngine::new_from_config(
            &LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(16 * 1024 * 1024)
                .block_cache_size_mb(512)
                .build()
                .unwrap(),
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        let keys_per_batch = 100_000;
        for batch in 0..10 {
            for i in batch * keys_per_batch..(batch + 1) * keys_per_batch {
                let key = generate_key(i, 10);
                let value = generate_value(i, 100);
                engine.set(key, value).unwrap();
            }
            engine.flush_memtable().unwrap();
        }

        let benchmark_keys: Vec<String> = (0..10_000).map(|i| generate_key(i, 10)).collect();

        b.iter(|| {
            for key in benchmark_keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });

        drop(engine);
        drop(temp_dir);
    });

    group.finish();
}

/// Benchmark concurrent read/write access
fn bench_concurrent_access(c: &mut Criterion) {
    use std::sync::{Arc, Mutex};

    let thread_count: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![1]
    } else {
        vec![1, 2, 4]
    };
    for threads in thread_count {
        let mut group = c.benchmark_group(format!("concurrent_{}_threads", threads));

        group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, _t| {
            let (temp_dir, data_dir) = setup_temp_dir("concurrent");
            let engine = apexstore::LsmEngine::new_from_config(
                &LsmConfig::builder()
                    .dir_path(data_dir.clone())
                    .memtable_max_size(16 * 1024 * 1024)
                    .block_cache_size_mb(256)
                    .build()
                    .unwrap(),
                apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            for i in 0..10_000 {
                let key = generate_key(i, 10);
                let value = generate_value(i, 100);
                engine.set(key, value).unwrap();
            }

            let engine = Arc::new(engine);
            let keys: Vec<String> = (0..10_000).map(|i| generate_key(i, 10)).collect();
            let key_set: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(keys));

            b.iter(|| {
                let keys_clone = key_set.lock().unwrap();
                let mut handles = Vec::new();

                for _ in 0..threads {
                    let keys_subset: Vec<String> = keys_clone.iter().cloned().collect();
                    let engine_clone = Arc::clone(&engine);
                    let handle = std::thread::spawn(move || {
                        for key in keys_subset.iter() {
                            let _ = engine_clone.get(key.as_str()).unwrap();
                        }
                    });
                    handles.push(handle);
                }

                for handle in handles {
                    handle.join().unwrap();
                }
            });

            drop(engine);
            drop(temp_dir);
        });

        group.finish();
    }
}

/// Benchmark with memory pressure (small memtable)
fn bench_memory_pressure(c: &mut Criterion) {
    let num_keys = if std::env::var("CI").is_ok() {
        10_000usize
    } else {
        100_000usize
    };
    let mut group = c.benchmark_group("memory_pressure");

    group.bench_with_input(
        BenchmarkId::from_parameter("small_memtable"),
        &(),
        |b, &_| {
            let (temp_dir, data_dir) = setup_temp_dir("memory_pressure");
            let engine = apexstore::LsmEngine::new_from_config(
                &LsmConfig::builder()
                    .dir_path(data_dir.clone())
                    .memtable_max_size(2 * 1024 * 1024)
                    .build()
                    .unwrap(),
                apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            for i in 0..num_keys {
                let key = generate_key(i, 10);
                let value = generate_value(i, 100);
                engine.set(key, value).unwrap();
            }

            let read_keys_count = if std::env::var("CI").is_ok() {
                1_000
            } else {
                10_000
            };
            let benchmark_keys: Vec<String> =
                (0..read_keys_count).map(|i| generate_key(i, 10)).collect();

            b.iter(|| {
                for key in benchmark_keys.iter() {
                    let _ = engine.get(key.as_str()).unwrap();
                }
            });

            drop(engine);
            drop(temp_dir);
        },
    );

    group.finish();
}

/// Benchmark with many SSTables (thousands of layers)
fn bench_many_sstables(c: &mut Criterion) {
    let sstable_counts: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![10]
    } else {
        vec![10, 50, 100]
    };
    for &sstable_count in &sstable_counts {
        let mut group = c.benchmark_group(format!("many_sstables_{}", sstable_count));

        group.bench_with_input(
            BenchmarkId::from_parameter(sstable_count),
            &(),
            |b, &_sc| {
                let (temp_dir, data_dir) = setup_temp_dir(&format!("many_sst_{}", sstable_count));
                let engine = apexstore::LsmEngine::new_from_config(
                    &LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(512 * 1024)
                        .build()
                        .unwrap(),
                    apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
                )
                .unwrap();

                let records_per_sstable = 1_000;
                for sstable in 0..sstable_count {
                    for i in (sstable * records_per_sstable)..((sstable + 1) * records_per_sstable)
                    {
                        let key = generate_key(i, 10);
                        let value = generate_value(i, 100);
                        engine.set(key, value).unwrap();
                    }
                    engine.flush_memtable().unwrap();
                }

                let benchmark_keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();

                b.iter(|| {
                    for key in benchmark_keys.iter() {
                        let _ = engine.get(key.as_str()).unwrap();
                    }
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

/// Benchmark cache thrashing scenario
fn bench_cache_thrashing(c: &mut Criterion) {
    let cache_sizes: Vec<usize> = if std::env::var("CI").is_ok() {
        vec![64]
    } else {
        vec![16, 64, 128]
    };
    for &cache_mb in &cache_sizes {
        let mut group = c.benchmark_group(format!("cache_thrash_{}MB", cache_mb));

        group.bench_with_input(BenchmarkId::from_parameter(cache_mb), &(), |b, &_cm| {
            let (temp_dir, data_dir) =
                setup_temp_dir(format!("cache_thrash_{}", cache_mb).as_str());
            let engine = apexstore::LsmEngine::new_from_config(
                &LsmConfig::builder()
                    .dir_path(data_dir.clone())
                    .memtable_max_size(16 * 1024 * 1024)
                    .block_cache_size_mb(cache_mb)
                    .build()
                    .unwrap(),
                apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            let total_keys = if std::env::var("CI").is_ok() {
                10_000
            } else {
                100_000
            };
            for i in 0..total_keys {
                let key = generate_key(i, 10);
                let value = generate_value(i, 100);
                engine.set(key, value).unwrap();
            }
            engine.flush_memtable().unwrap();

            let keys: Vec<String> = (0..total_keys).map(|i| generate_key(i, 10)).collect();

            b.iter(|| {
                for i in (0..10_000).step_by(50) {
                    let _ = engine.get(&keys[i]).unwrap();
                }
            });

            drop(engine);
            drop(temp_dir);
        });

        group.finish();
    }
}

/// Benchmark with duplicate key updates
fn bench_key_updates(c: &mut Criterion) {
    let mut group = c.benchmark_group("key_updates");

    group.bench_with_input(BenchmarkId::from_parameter("10k_keys"), &(), |b, &_| {
        let (temp_dir, data_dir) = setup_temp_dir("key_updates");
        let engine = apexstore::LsmEngine::new_from_config(
            &LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(64 * 1024 * 1024)
                .block_cache_size_mb(256)
                .build()
                .unwrap(),
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        let initial_keys: Vec<String> = (0..10_000).map(|i| generate_key(i, 10)).collect();

        for key in initial_keys.iter() {
            let value = generate_value(0, 100);
            engine.set(key.clone(), value).unwrap();
        }

        b.iter(|| {
            for (i, key) in initial_keys.iter().enumerate() {
                let value = generate_value(i, 100);
                engine.set(key.clone(), value).unwrap();
            }
        });

        drop(engine);
        drop(temp_dir);
    });

    group.finish();
}

/// Benchmark tombstone/delete operations
fn bench_delete_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("delete_operations");

    group.bench_with_input(BenchmarkId::from_parameter("10k_keys"), &(), |b, &_| {
        let (temp_dir, data_dir) = setup_temp_dir("delete_ops");
        let engine = apexstore::LsmEngine::new_from_config(
            &LsmConfig::builder()
                .dir_path(data_dir.clone())
                .memtable_max_size(64 * 1024 * 1024)
                .block_cache_size_mb(256)
                .build()
                .unwrap(),
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        let keys: Vec<String> = (0..10_000).map(|i| generate_key(i, 10)).collect();

        for key in keys.iter() {
            let value = generate_value(0, 100);
            engine.set(key.clone(), value).unwrap();
        }

        b.iter(|| {
            for key in keys.iter() {
                engine.delete(key.clone()).unwrap();
            }
        });

        drop(engine);
        drop(temp_dir);
    });

    group.finish();
}

criterion_group!(
    name = stress_benches;
    config = configure_criterion();
    targets = bench_large_dataset_1m, bench_concurrent_access, bench_memory_pressure, bench_many_sstables, bench_cache_thrashing, bench_key_updates, bench_delete_operations
);

criterion_main!(stress_benches);
