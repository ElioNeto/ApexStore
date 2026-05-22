use apexstore::infra::config::{CompactionStrategy, LsmConfig};
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::LsmEngine;
use criterion::{criterion_group, criterion_main, Criterion};
use tempfile::TempDir;

/// Setup a temporary directory for benchmark testing
fn setup_temp_dir() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join("write_amp");
    (temp_dir, path)
}

/// Generate deterministic test key
fn generate_key(index: usize) -> String {
    format!("key_{:08x}", index)
}

/// Compute write amplification ratio from the engine's compaction metrics.
fn compute_write_amplification(engine: &LsmEngine) -> Option<f64> {
    if let Ok(results) = engine.compact() {
        let mut total_read: u64 = 0;
        let mut total_written: u64 = 0;
        for (_cf, metrics) in &results {
            total_read += metrics.bytes_read;
            total_written += metrics.bytes_written;
        }
        if total_read > 0 {
            return Some(total_written as f64 / total_read as f64);
        }
    }
    None
}

/// Benchmark write amplification for Leveled compaction strategy
fn bench_write_amplification_leveled(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_amplification_leveled");
    group.sample_size(10);

    group.bench_function("leveled_10k_keys", |b| {
        b.iter(|| {
            let (_temp_dir, data_dir) = setup_temp_dir();
            let config = LsmConfig::builder()
                .dir_path(data_dir)
                .memtable_max_size(4096) // small memtable → many flushes
                .block_cache_size_mb(1)
                .strategy(CompactionStrategy::Leveled)
                .min_compaction_threshold(4)
                .max_sstables(16)
                .build()
                .unwrap();

            let engine =
                LsmEngine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();

            // Write enough keys to trigger multiple flushes and compactions
            let num_keys = 10_000usize;

            for i in 0..num_keys {
                let key = generate_key(i);
                let value = vec![b'y'; 50];
                engine.set(key, value).unwrap();
            }

            // Measure write amplification
            let wa = compute_write_amplification(&engine);
            if let Some(ratio) = wa {
                println!("  → Leveled write amplification: {ratio:.2}x (target < 10x)");
                assert!(
                    ratio < 10.0,
                    "Leveled write amplification too high: {ratio:.2}x (expected < 10x)"
                );
            } else {
                println!("  → Leveled: no compaction metrics available (all data in memtable)");
            }

            drop(engine);
        })
    });

    group.finish();
}

/// Benchmark write amplification for Size-Tiered compaction strategy
fn bench_write_amplification_size_tiered(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_amplification_size_tiered");
    group.sample_size(10);

    group.bench_function("size_tiered_10k_keys", |b| {
        b.iter(|| {
            let (_temp_dir, data_dir) = setup_temp_dir();
            let config = LsmConfig::builder()
                .dir_path(data_dir)
                .memtable_max_size(4096) // small memtable → many flushes
                .block_cache_size_mb(1)
                .strategy(CompactionStrategy::SizeTiered)
                .min_compaction_threshold(4)
                .max_sstables(16)
                .build()
                .unwrap();

            let engine =
                LsmEngine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();

            let num_keys = 10_000usize;

            for i in 0..num_keys {
                let key = generate_key(i);
                let value = vec![b'z'; 50];
                engine.set(key, value).unwrap();
            }

            // Measure write amplification
            let wa = compute_write_amplification(&engine);
            if let Some(ratio) = wa {
                println!("  → Size-Tiered write amplification: {ratio:.2}x (target < 3x)");
                assert!(
                    ratio < 3.0,
                    "Size-Tiered write amplification too high: {ratio:.2}x (expected < 3x)"
                );
            } else {
                println!("  → Size-Tiered: no compaction metrics available (all data in memtable)");
            }

            drop(engine);
        })
    });

    group.finish();
}

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

criterion_group!(
    name = write_amplification;
    config = configure_criterion();
    targets = bench_write_amplification_leveled, bench_write_amplification_size_tiered
);

criterion_main!(write_amplification);
