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

            let mut engine = LsmEngine::new_from_config(
                &config,
                GlobalBlockCache::new(1, 4096),
            )
            .unwrap();

            // Write enough keys to trigger multiple flushes and compactions
            let num_keys = 10_000usize;
            let input_bytes: usize = num_keys * (16 + 50); // key ~16 + value ~50

            for i in 0..num_keys {
                let key = generate_key(i);
                let value = vec![b'y'; 50];
                engine.set(key, value).unwrap();
            }

            // Get compaction metrics for write amplification calculation
            if let Ok(results) = engine.compact() {
                for (_cf, metrics) in &results {
                    if metrics.bytes_read > 0 {
                        let wa =
                            metrics.bytes_written as f64 / metrics.bytes_read as f64;
                        // Leveled should have < 10x amplification
                        assert!(
                            wa < 10.0,
                            "Leveled write amplification too high: {:.2}x (expected < 10x)",
                            wa
                        );
                    }
                }
            }

            // Also report input vs output ratio overall
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

            let mut engine = LsmEngine::new_from_config(
                &config,
                GlobalBlockCache::new(1, 4096),
            )
            .unwrap();

            let num_keys = 10_000usize;

            for i in 0..num_keys {
                let key = generate_key(i);
                let value = vec![b'z'; 50];
                engine.set(key, value).unwrap();
            }

            // Get compaction metrics
            if let Ok(results) = engine.compact() {
                for (_cf, metrics) in &results {
                    if metrics.bytes_read > 0 {
                        let wa =
                            metrics.bytes_written as f64 / metrics.bytes_read as f64;
                        // Size-Tiered should have < 3x amplification
                        assert!(
                            wa < 3.0,
                            "Size-Tiered write amplification too high: {:.2}x (expected < 3x)",
                            wa
                        );
                    }
                }
            }

            drop(engine);
        })
    });

    group.finish();
}

criterion_group!(
    name = write_amplification;
    config = Criterion::default()
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(1))
        .measurement_time(std::time::Duration::from_secs(3));
    targets = bench_write_amplification_leveled, bench_write_amplification_size_tiered
);

criterion_main!(write_amplification);
