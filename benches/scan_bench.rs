use apexstore::infra::config::LsmConfig;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
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
    let fill_count = remaining.min(64);
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', fill_count));
    value.truncate(value_size);
    value
}

fn bench_full_scan(c: &mut Criterion) {
    for num_keys in [1_000usize, 10_000, 100_000] {
        let mut group = c.benchmark_group("full_scan");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("full_scan");
                let engine = apexstore::LsmEngine::new(
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

                b.iter(|| {
                    let results = engine.scan().unwrap();
                    assert_eq!(results.len(), nk);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

fn bench_range_scan(c: &mut Criterion) {
    for scan_size in [100usize, 1_000, 10_000, 100_000] {
        let mut group = c.benchmark_group(format!("range_scan_{}", scan_size));
        group.throughput(Throughput::Elements(scan_size as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(scan_size),
            &scan_size,
            |b, &_ss| {
                let total_keys = 1_000_000usize;
                let (temp_dir, data_dir) = setup_temp_dir("range_scan");
                let engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(total_keys * 110 / 2)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..total_keys).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> =
                    (0..total_keys).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                let start_idx = total_keys / 2;
                let end_idx = start_idx + scan_size;
                let start_key = keys[start_idx].clone();
                let end_key = keys[end_idx - 1].clone();

                b.iter(|| {
                    let results = engine
                        .scan_range(
                            Some(start_key.as_str()),
                            Some(end_key.as_str()),
                            scan_size + 1000,
                        )
                        .unwrap();
                    assert!(results.0.len() >= scan_size / 2);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

fn bench_prefix_scan(c: &mut Criterion) {
    for prefix_size in [100usize, 1_000, 10_000] {
        let mut group = c.benchmark_group(format!("prefix_scan_{}", prefix_size));
        group.throughput(Throughput::Elements(prefix_size as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(prefix_size),
            &prefix_size,
            |b, &_ps| {
                let total_keys = 100_000usize;
                let (temp_dir, data_dir) = setup_temp_dir("prefix_scan");
                let engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(total_keys * 110 / 2)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                for i in 0..total_keys {
                    let key = format!("user:{}:data:{}", i / 100, i);
                    let value = generate_value(i, 100);
                    engine.set(key, value).unwrap();
                }

                let prefix = "user:5:data:";

                b.iter(|| {
                    let (results, _cursor) = engine
                        .search_prefix(prefix, None, prefix_size + 100)
                        .unwrap();
                    assert!(results.len() <= prefix_size + 100);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

fn bench_iteration_sorted(c: &mut Criterion) {
    for num_keys in [1_000usize, 10_000, 100_000] {
        let mut group = c.benchmark_group("iteration_sorted");
        group.throughput(Throughput::Elements(num_keys as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_keys),
            &num_keys,
            |b, &nk| {
                let (temp_dir, data_dir) = setup_temp_dir("iteration_sorted");
                let engine = apexstore::LsmEngine::new(
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

                b.iter(|| {
                    let results = engine.scan().unwrap();
                    for i in 1..results.len() {
                        assert!(results[i - 1].0 <= results[i].0);
                    }
                    assert_eq!(results.len(), nk);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

fn bench_scan_with_limit(c: &mut Criterion) {
    for limit in [10usize, 100, 1_000, 10_000] {
        let mut group = c.benchmark_group(format!("scan_limit_{}", limit));
        group.throughput(Throughput::Elements(limit as u64));

        group.bench_with_input(BenchmarkId::from_parameter(limit), &limit, |b, &_l| {
            let total_keys = 1_000_000usize;
            let (temp_dir, data_dir) = setup_temp_dir("scan_limit");
            let engine = apexstore::LsmEngine::new(
                LsmConfig::builder()
                    .dir_path(data_dir.clone())
                    .memtable_max_size(total_keys * 110 / 2)
                    .build()
                    .unwrap(),
            )
            .unwrap();

            let keys: Vec<String> = (0..total_keys).map(|i| generate_key(i, 10)).collect();
            let values: Vec<Vec<u8>> = (0..total_keys).map(|i| generate_value(i, 100)).collect();

            for (key, value) in keys.iter().zip(values.iter()) {
                engine.set(key.clone(), value.clone()).unwrap();
            }

            b.iter(|| {
                let results = engine.scan_range(None, None, limit).unwrap();
                assert!(results.0.len() <= limit);
            });

            drop(engine);
            drop(temp_dir);
        });

        group.finish();
    }
}

fn bench_scan_pagination(c: &mut Criterion) {
    for num_pages in [10usize, 100, 1_000] {
        let mut group = c.benchmark_group("scan_pagination");
        group.throughput(Throughput::Elements((num_pages * 100) as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(num_pages),
            &num_pages,
            |b, &_np| {
                let total_keys = 100_000usize;
                let page_size = 100usize;
                let (temp_dir, data_dir) = setup_temp_dir("scan_pagination");
                let engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(total_keys * 110 / 2)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                let keys: Vec<String> = (0..total_keys).map(|i| generate_key(i, 10)).collect();
                let values: Vec<Vec<u8>> =
                    (0..total_keys).map(|i| generate_value(i, 100)).collect();

                for (key, value) in keys.iter().zip(values.iter()) {
                    engine.set(key.clone(), value.clone()).unwrap();
                }

                b.iter(|| {
                    let mut cursor: Option<String> = None;
                    let mut fetched = 0usize;
                    while fetched < num_pages * page_size && fetched <= total_keys {
                        let results = engine
                            .scan_range(cursor.as_deref(), None, page_size)
                            .unwrap();
                        if results.0.is_empty() {
                            break;
                        }
                        fetched += results.0.len();
                        cursor = results.1;
                        if fetched >= num_pages * page_size {
                            break;
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

fn bench_sstable_layer_scan(c: &mut Criterion) {
    for layer_count in [1usize, 3, 10, 30] {
        let mut group = c.benchmark_group(format!("sstable_layer_{}", layer_count));

        group.bench_with_input(
            BenchmarkId::from_parameter(layer_count),
            &layer_count,
            |b, &_lc| {
                let keys_per_layer = 10_000usize;
                let (temp_dir, data_dir) = setup_temp_dir("sstable_layer");
                let engine = apexstore::LsmEngine::new(
                    LsmConfig::builder()
                        .dir_path(data_dir.clone())
                        .memtable_max_size(1024 * 1024)
                        .build()
                        .unwrap(),
                )
                .unwrap();

                for layer in 0..layer_count {
                    for i in 0..keys_per_layer {
                        let key = format!("layer{}_key{:08x}", layer, i);
                        let value = generate_value(i, 100);
                        engine.set(key, value).unwrap();
                    }
                    engine.flush_memtable().unwrap();
                }

                b.iter(|| {
                    let results = engine.scan().unwrap();
                    assert!(results.len() >= keys_per_layer);
                });

                drop(engine);
                drop(temp_dir);
            },
        );

        group.finish();
    }
}

criterion_group!(
    scan_benches,
    bench_full_scan,
    bench_range_scan,
    bench_prefix_scan,
    bench_iteration_sorted,
    bench_scan_with_limit,
    bench_scan_pagination,
    bench_sstable_layer_scan,
);

criterion_main!(scan_benches);
