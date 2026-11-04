use apexstore::{
    core::log_record::LogRecord,
    infra::config::{LsmConfig, StorageConfig},
    storage::{
        builder::SstableBuilder, cache::GlobalBlockCache, encryption::EncryptionConfig,
        reader::SstableReader, wal::WriteAheadLog,
    },
    LsmEngine, LsmError,
};
use std::{
    path::Path,
    process::{Command, Output},
};
fn legacy() -> EncryptionConfig {
    EncryptionConfig {
        enabled: true,
        key: [0; 32],
    }
}
fn target() -> EncryptionConfig {
    EncryptionConfig {
        enabled: true,
        key: [0x41; 32],
    }
}
fn cli(source: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_apexstore-cli"))
        .arg("--db")
        .arg(source)
        .args(args)
        .output()
        .unwrap()
}
fn make_legacy(source: &Path) -> Vec<LogRecord> {
    std::fs::create_dir_all(source.join("sstables")).unwrap();
    let mut live = LogRecord::new(b"live".to_vec(), b"retained".to_vec());
    live.timestamp = 10;
    live.expires_at = Some(u128::MAX);
    live.column_family = Some("other".into());
    let mut deleted = LogRecord::tombstone(b"deleted".to_vec());
    deleted.timestamp = 20;
    deleted.column_family = Some("other".into());
    let mut range = LogRecord::range_tombstone(b"range".to_vec(), b"z".to_vec());
    range.timestamp = 30;
    range.column_family = Some("other".into());
    let records = vec![deleted, live, range];
    let mut builder = SstableBuilder::new_with_encryption(
        source.join("sstables/legacy.sst"),
        StorageConfig {
            block_size: 128,
            ..StorageConfig::default()
        },
        123,
        &legacy(),
    )
    .unwrap();
    for record in &records {
        builder.add(&record.key, record).unwrap();
    }
    builder.finish().unwrap();
    let reader = SstableReader::open_with_encryption(
        source.join("sstables/legacy.sst"),
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        &legacy(),
    )
    .unwrap();
    assert!(
        reader.metadata().blocks.len() > 1,
        "exercise migration across block boundaries"
    );
    std::fs::write(
        source.join("disk.sst.manifest"),
        br#"{"column_families":{"other":["legacy.sst"]}}"#,
    )
    .unwrap();
    let wal = WriteAheadLog::new_with_encryption(source, "default", &legacy()).unwrap();
    for value in [b"old".as_slice(), b"new"] {
        let mut record = LogRecord::new(b"wal".to_vec(), value.to_vec());
        record.timestamp = 40;
        wal.write_record(&record).unwrap();
    }
    wal.sync().unwrap();
    let mut other = LogRecord::new(b"pending".to_vec(), b"private".to_vec());
    other.column_family = Some("other".into());
    other.timestamp = 50;
    let wal = WriteAheadLog::new_with_encryption(source, "other", &legacy()).unwrap();
    wal.write_record(&other).unwrap();
    wal.sync().unwrap();
    std::fs::create_dir(source.join("credentials")).unwrap();
    std::fs::write(
        source.join("credentials/tokens.json"),
        b"private metadata preserved byte for byte",
    )
    .unwrap();
    records
}
fn source_bytes(source: &Path) -> Vec<Vec<u8>> {
    [
        "sstables/legacy.sst",
        "wal.log",
        "wal-other.log",
        "disk.sst.manifest",
        "credentials/tokens.json",
    ]
    .iter()
    .map(|name| std::fs::read(source.join(name)).unwrap())
    .collect()
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
#[test]
fn real_cli_migrates_zero_key_sstables_and_all_wal_frames_to_key_or_plaintext() {
    for encrypted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let output = dir.path().join("output");
        let expected = make_legacy(&source);
        let original = source_bytes(&source);
        let key_path = dir.path().join("target.key");
        std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
        let mut args = vec![
            "migrate-encryption",
            "--legacy-zero-key",
            "--output",
            output.to_str().unwrap(),
        ];
        if encrypted {
            args.extend(["--target-key-file", key_path.to_str().unwrap()]);
        } else {
            args.push("--plaintext");
        }
        let result = cli(&source, &args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["sstables"], 1);
        assert_eq!(
            report["wal_frames"], 3,
            "must preserve duplicate-key frames, not deduplicate"
        );
        assert_eq!(source_bytes(&source), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for database in [&source, &output] {
                assert_eq!(
                    std::fs::metadata(database.join(".apexstore.lock"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        let encryption = if encrypted {
            target()
        } else {
            EncryptionConfig::default()
        };
        let bytes = std::fs::read(output.join("sstables/legacy.sst")).unwrap();
        assert_eq!(
            &bytes[..8],
            if encrypted { b"LSMSST04" } else { b"LSMSST03" }
        );
        let reader = SstableReader::open_with_encryption(
            output.join("sstables/legacy.sst"),
            StorageConfig::default(),
            GlobalBlockCache::new(1, 4096),
            &encryption,
        )
        .unwrap();
        assert_eq!(reader.metadata().timestamp, 123);
        assert_eq!(
            reader
                .scan()
                .unwrap()
                .into_iter()
                .map(|(_, r)| r)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            std::fs::read(output.join("wal.log")).unwrap()[4],
            if encrypted { 3 } else { 2 }
        );
        assert_eq!(
            std::fs::read(output.join("credentials/tokens.json")).unwrap(),
            original[4]
        );
        let mut cfg = LsmConfig::default();
        cfg.core.dir_path = output;
        cfg.storage.encryption_enabled = encrypted;
        cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
        let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
        assert_eq!(engine.get("wal").unwrap(), Some(b"new".to_vec()));
        assert_eq!(
            engine.get_cf("other", b"pending").unwrap(),
            Some(b"private".to_vec())
        );
        assert_eq!(
            engine.get_cf("other", b"live").unwrap(),
            Some(b"retained".to_vec())
        );
        assert_eq!(engine.get_cf("other", b"deleted").unwrap(), None);
    }
}

#[test]
fn legacy_detection_is_actionable_and_never_moves_files() {
    for wal in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        make_legacy(&source);
        if !wal {
            std::fs::remove_file(source.join("wal.log")).unwrap();
            std::fs::remove_file(source.join("wal-other.log")).unwrap();
        }
        let table = std::fs::read(source.join("sstables/legacy.sst")).unwrap();
        let mut cfg = LsmConfig::default();
        cfg.core.dir_path = source.clone();
        let result = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096));
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("legacy data must require explicit migration"),
        };
        assert!(matches!(error, LsmError::LegacyEncryptionMigrationRequired));
        assert!(error.to_string().contains("migrate-encryption"));
        assert_eq!(
            std::fs::read(source.join("sstables/legacy.sst")).unwrap(),
            table
        );
        assert!(!source.join("sstables/quarantine").exists());
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
#[test]
fn migration_rejects_missing_authorization_corruption_and_existing_outputs() {
    for failure in [
        "missing-legacy",
        "wrong-key",
        "bad-crc",
        "truncated-wal",
        "existing-output",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let output = dir.path().join("output");
        make_legacy(&source);
        if failure == "bad-crc" {
            let path = source.join("wal.log");
            let mut bytes = std::fs::read(&path).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 1;
            std::fs::write(path, bytes).unwrap();
        }
        if failure == "truncated-wal" {
            let path = source.join("wal.log");
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.pop();
            std::fs::write(path, bytes).unwrap();
        }
        let before = source_bytes(&source);
        if failure == "existing-output" {
            std::fs::create_dir(&output).unwrap();
            std::fs::write(output.join("keep"), b"retained").unwrap();
        }
        let mut args = vec![
            "migrate-encryption",
            "--output",
            output.to_str().unwrap(),
            "--plaintext",
        ];
        let wrong = dir.path().join("wrong.key");
        std::fs::write(&wrong, hex::encode([0x42; 32])).unwrap();
        if failure == "wrong-key" {
            args.extend(["--source-key-file", wrong.to_str().unwrap()]);
        } else if failure != "missing-legacy" {
            args.push("--legacy-zero-key");
        }
        let result = cli(&source, &args);
        assert!(!result.status.success(), "{failure}");
        assert_eq!(source_bytes(&source), before);
        if failure == "existing-output" {
            assert_eq!(std::fs::read(output.join("keep")).unwrap(), b"retained");
        } else {
            assert!(!output.exists());
        }
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".apexstore-migration-")));
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
#[test]
fn migration_respects_the_live_engine_lock_and_keeps_existing_cli_commands() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let output = dir.path().join("output");
    assert!(cli(&source, &["set", "key", "value"]).status.success());
    let mut cfg = LsmConfig::default();
    cfg.core.dir_path = source.clone();
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    let result = cli(
        &source,
        &[
            "migrate-encryption",
            "--output",
            output.to_str().unwrap(),
            "--plaintext",
        ],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("lock unavailable"));
    assert!(!output.exists());
    engine.close();
    drop(engine);
    let result = cli(&source, &["get", "key"]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("value"));
}

#[test]
fn scrubber_distinguishes_missing_key_from_real_corruption_and_verifies_with_key() {
    use apexstore::infra::health::scrubber::{scrub_file, scrub_file_with_encryption};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("encrypted.sst");
    let mut builder =
        SstableBuilder::new_with_encryption(path.clone(), StorageConfig::default(), 1, &target())
            .unwrap();
    builder
        .add(b"key", &LogRecord::new(b"key".to_vec(), b"value".to_vec()))
        .unwrap();
    builder.finish().unwrap();
    let skipped = scrub_file(&path);
    assert!(skipped.skipped);
    assert!(!skipped.valid);
    assert_eq!(skipped.corrupt_blocks, 0);
    let result = scrub_file_with_encryption(&path, &target());
    assert!(result.valid);
    assert!(!result.skipped);
    assert_eq!(result.verified_blocks, 1);
    let reader = SstableReader::open_with_encryption(
        path.clone(),
        StorageConfig::default(),
        GlobalBlockCache::new(1, 4096),
        &target(),
    )
    .unwrap();
    let block = &reader.metadata().blocks[0];
    let start = block.offset as usize;
    let end = start + block.size as usize - 4;
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[start] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    let damaged = scrub_file_with_encryption(&path, &target());
    assert!(!damaged.valid);
    assert!(!damaged.skipped);
    assert_eq!(damaged.corrupt_blocks, 1);
    // Recompute CRC to ensure authentication, not just checksum, rejects damage.
    let checksum = crc32fast::hash(&bytes[start..end]);
    bytes[end..end + 4].copy_from_slice(&checksum.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    let forged = scrub_file_with_encryption(&path, &target());
    assert!(!forged.valid);
    assert!(!forged.skipped);
    assert_eq!(forged.corrupt_blocks, 1);
    assert_eq!(forged.verified_blocks, 0);
    assert!(forged
        .errors
        .iter()
        .any(|error| error.contains("Block authentication failed")));
}

#[test]
fn configured_wrong_key_reports_legacy_migration_instead_of_quarantining_data() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    make_legacy(&source);
    let before = source_bytes(&source);
    let key_path = dir.path().join("configured.key");
    std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
    let mut cfg = LsmConfig::default();
    cfg.core.dir_path = source.clone();
    cfg.storage.encryption_enabled = true;
    cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
    assert!(matches!(
        LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)),
        Err(LsmError::LegacyEncryptionMigrationRequired)
    ));
    assert_eq!(source_bytes(&source), before);
    let result = cli(&source, &["stats"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("migrate-encryption"));
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
#[test]
fn migration_refuses_a_live_nested_database() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let output = dir.path().join("output");
    let mut cfg = LsmConfig::default();
    cfg.core.dir_path = source.join("nested");
    let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
    engine.set(b"key".to_vec(), b"retained".to_vec()).unwrap();
    let result = cli(
        &source,
        &[
            "migrate-encryption",
            "--output",
            output.to_str().unwrap(),
            "--plaintext",
        ],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("stop nested database"));
    assert!(!output.exists());
    assert_eq!(engine.get("key").unwrap(), Some(b"retained".to_vec()));
}

#[cfg(unix)]
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
#[test]
fn migration_refuses_symlinks_and_key_files_inside_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    make_legacy(&source);
    let output = dir.path().join("output");
    let key = source.join("secret.key");
    std::fs::write(&key, hex::encode([0x41; 32])).unwrap();
    let result = cli(
        &source,
        &[
            "migrate-encryption",
            "--legacy-zero-key",
            "--output",
            output.to_str().unwrap(),
            "--target-key-file",
            key.to_str().unwrap(),
        ],
    );
    assert!(!result.status.success());
    assert!(!output.exists());
    std::fs::remove_file(key).unwrap();
    let external = dir.path().join("outside");
    std::fs::write(&external, b"do not modify").unwrap();
    std::os::unix::fs::symlink(&external, source.join("symlink")).unwrap();
    let result = cli(
        &source,
        &[
            "migrate-encryption",
            "--legacy-zero-key",
            "--output",
            output.to_str().unwrap(),
            "--plaintext",
        ],
    );
    assert!(!result.status.success());
    assert!(!output.exists());
    assert_eq!(std::fs::read(external).unwrap(), b"do not modify");
}
