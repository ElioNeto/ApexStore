use apexstore::{
    core::{log_record::LogRecord, table::Table},
    infra::{
        codec::{decode, encode},
        config::{LsmConfig, StorageConfig},
    },
    storage::{
        builder::{MetaBlock, SstableBuilder},
        cache::GlobalBlockCache,
        encryption::EncryptionConfig,
        reader::SstableReader,
    },
    LsmEngine,
};
use std::path::Path;

fn plaintext() -> EncryptionConfig {
    EncryptionConfig {
        enabled: false,
        key: [0; 32],
    }
}
fn write_table(path: &Path, key: &str, encryption: &EncryptionConfig) {
    let mut builder = SstableBuilder::new_with_encryption(
        path.to_path_buf(),
        StorageConfig::default(),
        1,
        encryption,
    )
    .unwrap();
    builder
        .add(
            key.as_bytes(),
            &LogRecord::new(key.as_bytes().to_vec(), b"persisted".to_vec()),
        )
        .unwrap();
    builder.finish().unwrap();
}
fn break_bloom(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    let offset = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap()) as usize;
    let mut meta: MetaBlock =
        decode(&lz4_flex::decompress_size_prepended(&bytes[offset..bytes.len() - 8]).unwrap())
            .unwrap();
    meta.bloom_filter_data = vec![0xff; 3];
    let mut changed = bytes[..offset].to_vec();
    changed.extend(lz4_flex::compress_prepend_size(&encode(&meta).unwrap()));
    changed.extend((offset as u64).to_le_bytes());
    std::fs::write(path, changed).unwrap();
}
fn config(path: &Path) -> LsmConfig {
    let mut config = LsmConfig::default();
    config.core.dir_path = path.to_path_buf();
    config.storage.encryption_enabled = false;
    config
}

#[test]
fn unreadable_metadata_is_an_error_not_an_empty_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.sst");
    write_table(&path, "key", &plaintext());
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(16)
        .unwrap();
    let result = std::panic::catch_unwind(|| Table::from_sstable_path(&path, Some(&plaintext())));
    assert!(result.is_ok(), "corruption must not panic");
    assert!(
        result.unwrap().is_err(),
        "must not register an unreadable table"
    );
}

#[test]
fn invalid_footer_does_not_underflow_or_allocate_from_untrusted_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.sst");
    let mut bytes = b"LSMSST03".to_vec();
    bytes.extend(u64::MAX.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    let result = std::panic::catch_unwind(|| Table::from_sstable_path(&path, Some(&plaintext())));
    assert!(result.is_ok(), "invalid footer must not panic");
    assert!(result.unwrap().is_err());
}

#[test]
fn bad_bloom_keeps_real_range_and_readable_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("degraded.sst");
    write_table(&path, "key", &plaintext());
    break_bloom(&path);
    let table = Table::from_sstable_path(&path, Some(&plaintext()))
        .expect("Bloom failure must not discard data");
    assert_eq!(table.min_key, b"key");
    assert_eq!(table.max_key, b"key");
    assert!(table.bloom_filter.is_none());
    let reader = SstableReader::open_with_encryption(
        path,
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        &plaintext(),
    )
    .unwrap();
    assert!(reader.might_contain(b"key"));
    assert_eq!(reader.get(b"key").unwrap().unwrap().value, b"persisted");
    assert!(reader.get(b"absent").unwrap().is_none());
}

#[test]
fn plaintext_header_controls_decoding_even_when_a_key_is_available() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.sst");
    write_table(&path, "key", &plaintext());
    let key = EncryptionConfig {
        enabled: true,
        key: [0x41; 32],
    };
    let reader = SstableReader::open_with_encryption(
        path.clone(),
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        &key,
    )
    .unwrap();
    assert_eq!(reader.get(b"key").unwrap().unwrap().value, b"persisted");
    assert_eq!(
        Table::from_sstable_path(&path, Some(&key)).unwrap().min_key,
        b"key"
    );
}

#[test]
fn wrong_key_aborts_startup_without_moving_or_rewriting_tables() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    let path = sst.join("secret.sst");
    write_table(
        &path,
        "secret",
        &EncryptionConfig {
            enabled: true,
            key: [0x41; 32],
        },
    );
    let original = std::fs::read(&path).unwrap();
    let key_path = dir.path().join("wrong.key");
    std::fs::write(&key_path, hex::encode([0x42; 32])).unwrap();
    let mut cfg = config(dir.path());
    cfg.storage.encryption_enabled = true;
    cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
    assert!(
        LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).is_err(),
        "wrong key must fail startup"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(!sst.join("quarantine/secret.sst").exists());
}

#[test]
fn discovery_keeps_degraded_data_and_quarantines_only_structural_damage() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    write_table(&sst.join("healthy.sst"), "healthy", &plaintext());
    write_table(&sst.join("degraded.sst"), "degraded", &plaintext());
    break_bloom(&sst.join("degraded.sst"));
    std::fs::write(sst.join("corrupt.sst"), b"truncated").unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.get("healthy").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(engine.get("degraded").unwrap(), Some(b"persisted".to_vec()));
    assert!(sst.join("degraded.sst").exists());
    assert!(sst.join("quarantine/corrupt.sst").exists());
    let reason: serde_json::Value = serde_json::from_slice(
        &std::fs::read(sst.join("quarantine/corrupt.sst.quarantine-reason")).unwrap(),
    )
    .unwrap();
    assert!(reason["error"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(reason["engine_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(engine.stats("default").unwrap().quarantined_sstables, 1);
}

#[test]
fn missing_key_preflight_preserves_even_earlier_corrupt_files() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    std::fs::write(sst.join("a-corrupt.sst"), b"truncated").unwrap();
    write_table(
        &sst.join("z-encrypted.sst"),
        "secret",
        &EncryptionConfig {
            enabled: true,
            key: [0x41; 32],
        },
    );
    let original = std::fs::read(sst.join("z-encrypted.sst")).unwrap();
    let result = LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096));
    assert!(matches!(
        result,
        Err(apexstore::LsmError::EncryptionKeyRequired)
    ));
    assert_eq!(
        std::fs::read(sst.join("a-corrupt.sst")).unwrap(),
        b"truncated"
    );
    assert_eq!(
        std::fs::read(sst.join("z-encrypted.sst")).unwrap(),
        original
    );
    assert!(!sst.join("quarantine").exists());
}

#[test]
fn recovery_classifies_bloom_damage_and_authentication_separately() {
    use apexstore::core::table::TableLoad;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.sst");
    write_table(&path, "key", &plaintext());
    assert!(matches!(
        Table::load_for_recovery(&path, &plaintext()).unwrap(),
        TableLoad::Healthy(_)
    ));
    break_bloom(&path);
    assert!(matches!(
        Table::load_for_recovery(&path, &plaintext()).unwrap(),
        TableLoad::DegradedBloom(_)
    ));
    std::fs::write(&path, b"truncated").unwrap();
    assert!(matches!(
        Table::load_for_recovery(&path, &plaintext()).unwrap(),
        TableLoad::Corrupt(_)
    ));
    std::fs::remove_file(&path).unwrap();
    let key = EncryptionConfig {
        enabled: true,
        key: [0x41; 32],
    };
    write_table(&path, "key", &key);
    assert!(matches!(
        Table::load_for_recovery(
            &path,
            &EncryptionConfig {
                enabled: true,
                key: [0x42; 32]
            }
        ),
        Err(apexstore::LsmError::EncryptionAuthenticationFailed)
    ));
    let reader = SstableReader::open_with_encryption(
        path,
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        &key,
    )
    .unwrap();
    assert_eq!(reader.get(b"key").unwrap().unwrap().value, b"persisted");
}

#[test]
fn quarantine_evidence_survives_restart_and_filename_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    std::fs::write(sst.join("bad.sst"), b"first corruption").unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 1);
    engine.close();
    drop(engine);
    let reason_path = sst.join("quarantine/bad.sst.quarantine-reason");
    let original_reason = std::fs::read(&reason_path).unwrap();
    std::fs::write(sst.join("bad.sst"), b"second corruption").unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.stats("default").unwrap().quarantined_sstables, 2);
    assert_eq!(
        std::fs::read(sst.join("quarantine/bad.sst")).unwrap(),
        b"first corruption"
    );
    assert_eq!(std::fs::read(&reason_path).unwrap(), original_reason);
    let quarantined: Vec<_> = std::fs::read_dir(sst.join("quarantine"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sst"))
        .collect();
    assert_eq!(quarantined.len(), 2);
    assert!(quarantined
        .iter()
        .any(|p| std::fs::read(p).unwrap() == b"second corruption"));
    engine.close();
    drop(engine);
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 2);
}

#[test]
fn snapshot_restore_uses_the_same_degraded_and_quarantine_policy() {
    let source = tempfile::tempdir().unwrap();
    write_table(&source.path().join("healthy.sst"), "healthy", &plaintext());
    write_table(
        &source.path().join("degraded.sst"),
        "degraded",
        &plaintext(),
    );
    break_bloom(&source.path().join("degraded.sst"));
    std::fs::write(source.path().join("corrupt.sst"), b"truncated").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    engine.restore_snapshot(source.path()).unwrap();
    assert_eq!(engine.get("healthy").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(engine.get("degraded").unwrap(), Some(b"persisted".to_vec()));
    let stats = engine.stats("default").unwrap();
    assert_eq!(stats.quarantined_sstables, 1);
    assert_eq!(stats.unreadable_sstables, 1);
    assert_eq!(stats.degraded_bloom_sstables, 1);
    assert!(source.path().join("corrupt.sst").exists());
    assert!(dir
        .path()
        .join("sstables/quarantine/corrupt.sst.quarantine-reason")
        .exists());
    engine.close();
    drop(engine);
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(engine.get("degraded").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 1);
}

#[test]
fn snapshot_key_failure_does_not_copy_files_or_replace_the_live_wal() {
    let source = tempfile::tempdir().unwrap();
    write_table(&source.path().join("healthy.sst"), "healthy", &plaintext());
    write_table(
        &source.path().join("secret.sst"),
        "secret",
        &EncryptionConfig {
            enabled: true,
            key: [0x41; 32],
        },
    );
    std::fs::write(source.path().join("wal.log"), b"must not replace live WAL").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    engine
        .put_cf("default", b"live".to_vec(), b"retained".to_vec())
        .unwrap();
    let original_wal = std::fs::read(dir.path().join("wal.log")).unwrap();
    assert!(matches!(
        engine.restore_snapshot(source.path()),
        Err(apexstore::LsmError::EncryptionKeyRequired)
    ));
    assert_eq!(engine.get("live").unwrap(), Some(b"retained".to_vec()));
    assert_eq!(
        std::fs::read(dir.path().join("wal.log")).unwrap(),
        original_wal
    );
    assert!(!dir.path().join("sstables/healthy.sst").exists());
    assert!(!dir.path().join("sstables/secret.sst").exists());
}

#[actix_web::test]
async fn http_health_exposes_unreadable_data_but_bloom_degradation_stays_ready() {
    use actix_web::{http::StatusCode, test, web, App};
    for corrupt in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let sst = dir.path().join("sstables");
        std::fs::create_dir(&sst).unwrap();
        write_table(&sst.join("degraded.sst"), "degraded", &plaintext());
        break_bloom(&sst.join("degraded.sst"));
        if corrupt {
            std::fs::write(sst.join("corrupt.sst"), b"truncated").unwrap();
        }
        let engine =
            LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096))
                .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(engine))
                .app_data(web::Data::new(apexstore::api::config::AuthEnabled(false)))
                .configure(apexstore::api::configure),
        )
        .await;
        for route in ["/health/readiness", "/health/startup", "/health/check"] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(route).to_request()).await;
            assert_eq!(
                response.status(),
                if corrupt {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                },
                "{route}"
            );
            let body: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(body["details"]["degraded_bloom_sstables"], 1);
            if corrupt {
                assert_eq!(body["details"]["unreadable_sstables"], 1);
            }
        }
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/health/liveness")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        for route in ["/stats", "/stats/all"] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(route).to_request()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = test::read_body_json(response).await;
            let stats = if route == "/stats/all" {
                &body["data"]
            } else {
                &body
            };
            assert_eq!(stats["degraded_bloom_sstables"], 1);
            assert_eq!(stats["unreadable_sstables"], usize::from(corrupt));
            assert_eq!(stats["quarantined_sstables"], usize::from(corrupt));
        }
    }
}

#[test]
fn invalid_manifest_entries_fail_before_snapshot_files_are_copied() {
    for names in [
        vec!["../outside.sst"],
        vec!["missing.sst"],
        vec!["good.sst", "good.sst"],
    ] {
        let source = tempfile::tempdir().unwrap();
        write_table(&source.path().join("good.sst"), "good", &plaintext());
        std::fs::write(
            source.path().join("snapshot.manifest"),
            serde_json::to_vec(&serde_json::json!({"column_families":{"default":names}})).unwrap(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let engine =
            LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096))
                .unwrap();
        assert!(engine.restore_snapshot(source.path()).is_err());
        assert!(!dir.path().join("sstables/good.sst").exists());
        assert!(!dir.path().join("disk.sst.manifest").exists());
    }
}

#[test]
fn snapshot_manifest_preserves_column_family_and_quarantine_health_on_restart() {
    let source = tempfile::tempdir().unwrap();
    write_table(&source.path().join("good.sst"), "key", &plaintext());
    std::fs::write(source.path().join("bad.sst"), b"truncated").unwrap();
    std::fs::write(
        source.path().join("snapshot.manifest"),
        br#"{"column_families":{"other":["good.sst","bad.sst"]}}"#,
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    engine.restore_snapshot(source.path()).unwrap();
    assert_eq!(
        engine.get_cf("other", b"key").unwrap(),
        Some(b"persisted".to_vec())
    );
    engine.close();
    drop(engine);
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    assert_eq!(
        engine.get_cf("other", b"key").unwrap(),
        Some(b"persisted".to_vec())
    );
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 1);
}

#[test]
fn invalid_block_bounds_and_oversize_metadata_are_rejected_before_allocation() {
    use apexstore::core::table::TableLoad;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.sst");
    write_table(&path, "key", &plaintext());
    let bytes = std::fs::read(&path).unwrap();
    let offset = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap()) as usize;
    let metadata = lz4_flex::decompress_size_prepended(&bytes[offset..bytes.len() - 8]).unwrap();
    for (block_offset, block_size) in [(8, 0), (u64::MAX, 4), (offset as u64, 4)] {
        let mut meta: MetaBlock = decode(&metadata).unwrap();
        meta.blocks[0].offset = block_offset;
        meta.blocks[0].size = block_size;
        let mut bad = bytes[..offset].to_vec();
        bad.extend(lz4_flex::compress_prepend_size(&encode(&meta).unwrap()));
        bad.extend((offset as u64).to_le_bytes());
        std::fs::write(&path, bad).unwrap();
        assert!(matches!(
            Table::load_for_recovery(&path, &plaintext()).unwrap(),
            TableLoad::Corrupt(_)
        ));
    }
    let mut oversized = bytes[..offset].to_vec();
    oversized.extend(u32::MAX.to_le_bytes());
    oversized.extend((offset as u64).to_le_bytes());
    std::fs::write(&path, &oversized).unwrap();
    assert!(matches!(
        Table::load_for_recovery(&path, &plaintext()),
        Err(apexstore::LsmError::InvalidArgument(_))
    ));
    assert_eq!(std::fs::read(path).unwrap(), oversized);
}

#[test]
fn engine_uses_the_loaded_nonzero_key_for_discovered_table_reads() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    write_table(
        &sst.join("secret.sst"),
        "secret",
        &EncryptionConfig {
            enabled: true,
            key: [0x41; 32],
        },
    );
    write_table(&sst.join("plain.sst"), "plain", &plaintext());
    let key_path = dir.path().join("encryption.key");
    std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
    let mut cfg = config(dir.path());
    cfg.storage.encryption_enabled = true;
    cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    // The running engine must use its loaded key, without reloading a path.
    std::fs::remove_file(key_path).unwrap();
    assert_eq!(engine.get("secret").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(engine.get("plain").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(engine.stats("default").unwrap().unreadable_sstables, 0);
}

#[test]
fn snapshot_conflict_preserves_the_live_sstable_and_all_incoming_files() {
    let dir = tempfile::tempdir().unwrap();
    let sst = dir.path().join("sstables");
    std::fs::create_dir(&sst).unwrap();
    write_table(&sst.join("conflict.sst"), "live", &plaintext());
    let original = std::fs::read(sst.join("conflict.sst")).unwrap();
    let engine =
        LsmEngine::new_from_config(&config(dir.path()), GlobalBlockCache::new(1, 4096)).unwrap();
    let source = tempfile::tempdir().unwrap();
    write_table(
        &source.path().join("conflict.sst"),
        "incoming",
        &plaintext(),
    );
    write_table(&source.path().join("other.sst"), "other", &plaintext());
    assert!(matches!(
        engine.restore_snapshot(source.path()),
        Err(apexstore::LsmError::InvalidArgument(_))
    ));
    assert_eq!(std::fs::read(sst.join("conflict.sst")).unwrap(), original);
    assert!(!sst.join("other.sst").exists());
    assert_eq!(engine.get("live").unwrap(), Some(b"persisted".to_vec()));
}
