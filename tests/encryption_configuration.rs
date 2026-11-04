use apexstore::{
    core::engine::EngineOptions,
    infra::config::{LsmConfig, StorageConfig},
    storage::{cache::GlobalBlockCache, encryption::EncryptionConfig},
    LsmEngine,
};

#[test]
fn encryption_defaults_are_disabled_and_match_deserialization() {
    assert!(!EncryptionConfig::default().enabled);
    assert!(!EngineOptions::default().encryption.enabled);
    let config = StorageConfig::default();
    assert!(!config.encryption_enabled);
    let mut value = serde_json::to_value(&config).unwrap();
    value.as_object_mut().unwrap().remove("encryption_enabled");
    let decoded: StorageConfig = serde_json::from_value(value).unwrap();
    assert_eq!(decoded.encryption_enabled, config.encryption_enabled);
}

#[test]
fn explicit_encryption_requires_a_valid_nonzero_key_before_creating_a_database() {
    for contents in [
        None,
        Some("missing"),
        Some("not hex"),
        Some("41"),
        Some("0000000000000000000000000000000000000000000000000000000000000000"),
    ] {
        let parent = tempfile::tempdir().unwrap();
        let key_path = parent.path().join("encryption.key");
        if let Some(value) = contents {
            if value != "missing" {
                std::fs::write(&key_path, value).unwrap();
            }
        }
        let mut cfg = LsmConfig::default();
        cfg.core.dir_path = parent.path().join("database");
        cfg.storage.encryption_enabled = true;
        cfg.storage.encryption_key_path = contents.map(|_| key_path.to_str().unwrap().to_owned());
        let result = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096));
        assert!(
            result.is_err(),
            "must reject missing, unreadable, malformed and zero keys"
        );
        assert!(
            !cfg.core.dir_path.exists(),
            "configuration must fail before storage changes"
        );
    }
}

#[test]
fn encryption_debug_never_contains_key_material() {
    let cfg = EncryptionConfig {
        enabled: true,
        key: [0x41; 32],
    };
    let debug = format!("{cfg:?}");
    assert!(!debug.contains("65"));
    assert!(!debug.contains(&hex::encode(cfg.key)));
    assert!(debug.to_lowercase().contains("redacted"));
}

#[test]
fn serialized_layout_never_persists_or_accepts_runtime_key_material() {
    let config = apexstore::storage::config::StorageConfig {
        encryption: EncryptionConfig {
            enabled: true,
            key: [0x41; 32],
        },
        ..Default::default()
    };
    let mut value = serde_json::to_value(config).unwrap();
    assert!(value.get("encryption").is_none());
    value["encryption"] = serde_json::json!({"enabled":true,"key":vec![65;32]});
    assert!(serde_json::from_value::<apexstore::storage::config::StorageConfig>(value).is_err());
}

#[test]
fn losing_a_configured_key_rejects_restart_without_changing_sstables_or_wal() {
    let parent = tempfile::tempdir().unwrap();
    let key_path = parent.path().join("key");
    std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
    let mut cfg = LsmConfig::default();
    cfg.core.dir_path = parent.path().join("db");
    cfg.storage.encryption_enabled = true;
    cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    engine
        .set(b"flushed".to_vec(), b"sstable".to_vec())
        .unwrap();
    engine.flush_memtable().unwrap();
    engine.set(b"pending".to_vec(), b"wal".to_vec()).unwrap();
    engine.close();
    drop(engine);
    let table = std::fs::read_dir(cfg.core.dir_path.join("sstables"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "sst"))
        .unwrap();
    let sst = std::fs::read(&table).unwrap();
    let wal = std::fs::read(cfg.core.dir_path.join("wal.log")).unwrap();
    std::fs::remove_file(&key_path).unwrap();
    let result = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&table).unwrap(), sst);
    assert_eq!(
        std::fs::read(cfg.core.dir_path.join("wal.log")).unwrap(),
        wal
    );
    assert!(!cfg.core.dir_path.join("sstables/quarantine").exists());
    std::fs::write(key_path, hex::encode([0x41; 32])).unwrap();
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.get("flushed").unwrap(), Some(b"sstable".to_vec()));
    assert_eq!(engine.get("pending").unwrap(), Some(b"wal".to_vec()));
}

#[test]
fn direct_sstable_constructors_reject_enabled_encryption_without_a_key() {
    use apexstore::storage::{builder::SstableBuilder, reader::SstableReader};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.sst");
    let storage = StorageConfig {
        encryption_enabled: true,
        ..Default::default()
    };
    assert!(SstableBuilder::new(path.clone(), storage.clone(), 1).is_err());
    assert!(!path.exists());
    let mut builder = SstableBuilder::new(path.clone(), StorageConfig::default(), 1).unwrap();
    builder
        .add(
            b"key",
            &apexstore::core::log_record::LogRecord::new(b"key".to_vec(), b"value".to_vec()),
        )
        .unwrap();
    builder.finish().unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(SstableReader::open(path.clone(), storage, GlobalBlockCache::new(1, 4096)).is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn direct_sstable_constructors_honor_the_configured_key_and_disabled_mode() {
    use apexstore::storage::{builder::SstableBuilder, reader::SstableReader};
    for enabled in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.sst");
        let key = dir.path().join("key");
        std::fs::write(&key, hex::encode([0x41; 32])).unwrap();
        let storage = StorageConfig {
            encryption_enabled: enabled,
            encryption_key_path: Some(key.to_str().unwrap().to_owned()),
            ..Default::default()
        };
        let mut builder = SstableBuilder::new(path.clone(), storage.clone(), 1).unwrap();
        builder
            .add(
                b"key",
                &apexstore::core::log_record::LogRecord::new(b"key".to_vec(), b"value".to_vec()),
            )
            .unwrap();
        builder.finish().unwrap();
        assert_eq!(
            &std::fs::read(&path).unwrap()[..8],
            if enabled { b"LSMSST04" } else { b"LSMSST03" }
        );
        let reader = SstableReader::open(path, storage, GlobalBlockCache::new(1, 4096)).unwrap();
        assert_eq!(reader.get(b"key").unwrap().unwrap().value, b"value");
    }
}
