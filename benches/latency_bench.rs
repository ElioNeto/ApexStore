use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use criterion::{criterion_group, criterion_main, Criterion};
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
    let fill_count = remaining.min(64);
    let mut value = pattern.into_bytes();
    value.extend(std::iter::repeat_n(b'x', fill_count));
    value.truncate(value_size);
    value
}

fn bench_read_latency_memtable_1k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_memtable_1k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_mem_1k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &LsmConfig::builder()
            .dir_path(data_dir.clone())
            .memtable_max_size(1_000 * 220)
            .build()
            .unwrap(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }
    group.bench_function("memtable_1k", |b| {
        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });
    });
    group.finish();
    drop(engine);
    drop(temp_dir);
}

fn bench_read_latency_memtable_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_memtable_100k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_mem_100k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &LsmConfig::builder()
            .dir_path(data_dir.clone())
            .memtable_max_size(100_000 * 220)
            .build()
            .unwrap(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..100_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..100_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }
    group.bench_function("memtable_100k", |b| {
        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });
    });
    group.finish();
    drop(engine);
    drop(temp_dir);
}

fn bench_read_latency_sstable_1k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_sstable_1k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_sst_1k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &LsmConfig::builder()
            .dir_path(data_dir.clone())
            .memtable_max_size(1_000 * 110 / 2)
            .block_cache_size_mb(1)
            .build()
            .unwrap(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..1_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..1_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }
    group.bench_function("sstable_cold_1k", |b| {
        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });
    });
    group.finish();
    drop(engine);
    drop(temp_dir);
}

fn bench_read_latency_sstable_100k(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_latency_sstable_100k");
    let (temp_dir, data_dir) = setup_temp_dir("latency_sst_100k");
    let mut engine = apexstore::LsmEngine::new_from_config(
        &LsmConfig::builder()
            .dir_path(data_dir.clone())
            .memtable_max_size(100_000 * 110 / 2)
            .block_cache_size_mb(1)
            .build()
            .unwrap(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    let keys: Vec<String> = (0..100_000).map(|i| generate_key(i, 10)).collect();
    let values: Vec<Vec<u8>> = (0..100_000).map(|i| generate_value(i, 100)).collect();
    for (key, value) in keys.iter().zip(values.iter()) {
        engine.set(key.clone(), value.clone()).unwrap();
    }
    group.bench_function("sstable_cold_100k", |b| {
        b.iter(|| {
            for key in keys.iter() {
                let _ = engine.get(key.as_str()).unwrap();
            }
        });
    });
    group.finish();
    drop(engine);
    drop(temp_dir);
}

criterion_group!(
    name = latency_benches;
    config = configure_criterion();
    targets = bench_read_latency_memtable_1k, bench_read_latency_memtable_100k, bench_read_latency_sstable_1k, bench_read_latency_sstable_100k
);
criterion_main!(latency_benches);
