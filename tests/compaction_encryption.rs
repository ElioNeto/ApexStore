use apexstore::{
    core::{
        engine::{
            compaction::{
                CompactionStrategy, LazyLevelingCompaction, LeveledCompaction, SizeTieredCompaction,
            },
            EngineOptions,
        },
        table::Table,
    },
    infra::config::{LsmConfig, StorageConfig},
    storage::{cache::GlobalBlockCache, encryption::EncryptionConfig, reader::SstableReader},
    LsmEngine,
};
use std::{collections::BTreeMap, path::Path};

fn verify_table(path: &Path, encryption: &EncryptionConfig, key: &[u8], value: &[u8]) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(
        &bytes[..8],
        if encryption.enabled {
            b"LSMSST04"
        } else {
            b"LSMSST03"
        }
    );
    let reader = SstableReader::open_with_encryption(
        path.to_path_buf(),
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        encryption,
    )
    .unwrap();
    assert_eq!(reader.get(key).unwrap().unwrap().value, value);
}

fn engine_roundtrip(encrypted: bool) {
    let dir = tempfile::tempdir().unwrap();
    let key_path = dir.path().join("encryption.key");
    std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
    let mut cfg = LsmConfig::default();
    cfg.core.dir_path = dir.path().to_path_buf();
    cfg.core.memtable_max_size = 1024 * 1024;
    cfg.compaction.min_compaction_threshold = 4;
    cfg.storage.encryption_enabled = encrypted;
    cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
    let encryption = EncryptionConfig {
        enabled: encrypted,
        key: [0x41; 32],
    };
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    // The running engine owns the loaded key; compaction must not reload it.
    std::fs::remove_file(&key_path).unwrap();
    for i in 0..4 {
        engine
            .set(format!("key{i}").into_bytes(), b"persisted".to_vec())
            .unwrap();
        engine.flush_memtable().unwrap();
    }
    assert_eq!(engine.stats("default").unwrap().sst_files, 4);
    let results = engine.compact().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1.files_merged, 4);
    assert_eq!(engine.stats("default").unwrap().sst_files, 1);
    let paths: Vec<_> = std::fs::read_dir(dir.path().join("sstables"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sst"))
        .collect();
    assert_eq!(paths.len(), 1);
    for i in 0..4 {
        verify_table(
            &paths[0],
            &encryption,
            format!("key{i}").as_bytes(),
            b"persisted",
        );
    }
    engine.close();
    drop(engine);
    std::fs::write(key_path, hex::encode([0x41; 32])).unwrap();
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    for i in 0..4 {
        assert_eq!(
            engine.get(format!("key{i}")).unwrap(),
            Some(b"persisted".to_vec())
        );
    }
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 0);
}

#[test]
fn nonzero_key_survives_flush_compaction_and_restart() {
    engine_roundtrip(true);
}
#[test]
fn disabled_encryption_stays_plaintext_after_compaction() {
    engine_roundtrip(false);
}

#[test]
fn every_builtin_strategy_uses_runtime_encryption_instead_of_reloading_storage_config() {
    for enabled in [false, true] {
        let strategies: Vec<Box<dyn CompactionStrategy>> = vec![
            Box::new(SizeTieredCompaction::default()),
            Box::new(LeveledCompaction::default()),
            Box::new(LazyLevelingCompaction::default()),
        ];
        for strategy in strategies {
            let dir = tempfile::tempdir().unwrap();
            let options = EngineOptions {
                encryption: EncryptionConfig {
                    enabled,
                    key: [0x41; 32],
                },
                ..Default::default()
            };
            let storage = StorageConfig {
                encryption_enabled: !enabled,
                encryption_key_path: Some(
                    dir.path().join("missing.key").to_str().unwrap().to_owned(),
                ),
                ..Default::default()
            };
            let mut values = BTreeMap::new();
            values.insert(b"key".to_vec(), b"value".to_vec());
            let table = Table::build(values, &options);
            let (tables, metrics) = strategy
                .execute(vec![table], &options, &storage, dir.path(), &[])
                .unwrap();
            assert_eq!(tables.len(), 1, "{}", strategy.name());
            assert_eq!(metrics.files_merged, 1);
            verify_table(
                tables[0].path.as_ref().unwrap(),
                &options.encryption,
                b"key",
                b"value",
            );
        }
    }
}
