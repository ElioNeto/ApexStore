use apexstore::{
    core::log_record::LogRecord,
    infra::config::LsmConfig,
    storage::{
        cache::GlobalBlockCache,
        encryption::{EncryptionConfig, Encryptor},
        wal::WriteAheadLog,
    },
    LsmEngine, LsmError,
};
use std::path::Path;

fn key() -> EncryptionConfig {
    EncryptionConfig {
        enabled: true,
        key: [0x41; 32],
    }
}
fn plaintext() -> EncryptionConfig {
    EncryptionConfig {
        enabled: false,
        key: [0; 32],
    }
}
fn write(dir: &Path, cf: &str, encryption: &EncryptionConfig, record: &LogRecord) {
    let wal = WriteAheadLog::new_with_encryption(dir, cf, encryption).unwrap();
    wal.write_record(record).unwrap();
    wal.sync().unwrap();
}

#[test]
fn recovery_never_returns_partial_records_when_an_encrypted_frame_cannot_be_authenticated() {
    for encryption in [
        plaintext(),
        EncryptionConfig {
            enabled: true,
            key: [0x42; 32],
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "default",
            &plaintext(),
            &LogRecord::new(b"plain".to_vec(), b"retained".to_vec()),
        );
        write(
            dir.path(),
            "default",
            &key(),
            &LogRecord::new(b"secret".to_vec(), b"retained".to_vec()),
        );
        let original = std::fs::read(dir.path().join("wal.log")).unwrap();
        let wal = WriteAheadLog::new_with_encryption(dir.path(), "default", &encryption).unwrap();
        let result = wal.recover();
        if encryption.enabled {
            assert!(matches!(
                result,
                Err(LsmError::EncryptionAuthenticationFailed)
            ));
        } else {
            assert!(matches!(result, Err(LsmError::EncryptionKeyRequired)));
        }
        assert_eq!(std::fs::read(dir.path().join("wal.log")).unwrap(), original);
        drop(wal);
        let wal = WriteAheadLog::new_with_encryption(dir.path(), "default", &key()).unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].key, b"plain");
        assert_eq!(recovered[1].key, b"secret");
        assert!(recovered.iter().all(|r| r.value == b"retained"));
    }
}

#[test]
fn engine_startup_fails_with_missing_or_wrong_wal_key_in_every_column_family() {
    for cf in ["default", "private"] {
        for wrong_key in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut record = LogRecord::new(b"secret".to_vec(), b"retained".to_vec());
            record.column_family = Some(cf.to_owned());
            write(dir.path(), cf, &key(), &record);
            let filename = if cf == "default" {
                "wal.log"
            } else {
                "wal-private.log"
            };
            let original = std::fs::read(dir.path().join(filename)).unwrap();
            let key_path = dir.path().join("test.key");
            std::fs::write(&key_path, hex::encode([0x42; 32])).unwrap();
            let mut cfg = LsmConfig::default();
            cfg.core.dir_path = dir.path().to_path_buf();
            cfg.storage.encryption_enabled = wrong_key;
            cfg.storage.encryption_key_path = Some(key_path.to_str().unwrap().to_owned());
            let result = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096));
            if wrong_key {
                assert!(
                    matches!(result, Err(LsmError::EncryptionAuthenticationFailed)),
                    "{cf}"
                );
            } else {
                assert!(
                    matches!(result, Err(LsmError::EncryptionKeyRequired)),
                    "{cf}"
                );
            }
            assert_eq!(std::fs::read(dir.path().join(filename)).unwrap(), original);
            std::fs::write(&key_path, hex::encode([0x41; 32])).unwrap();
            cfg.storage.encryption_enabled = true;
            let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
            assert_eq!(
                engine.get_cf(cf, b"secret").unwrap(),
                Some(b"retained".to_vec())
            );
        }
    }
}

fn encrypted_frame(payload: &[u8]) -> Vec<u8> {
    let ciphertext = Encryptor::new(&key()).encrypt_block(payload).unwrap();
    let mut frame = ((ciphertext.len() + 1) as u32).to_le_bytes().to_vec();
    frame.push(3);
    frame.extend(ciphertext);
    let checksum = crc32fast::hash(&frame);
    frame.extend(checksum.to_le_bytes());
    frame
}

#[test]
fn authenticated_but_undecodable_wal_record_is_an_error_and_preserves_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let frame = encrypted_frame(&[0xff; 3]);
    std::fs::write(dir.path().join("wal.log"), &frame).unwrap();
    let wal = WriteAheadLog::new_with_encryption(dir.path(), "default", &key()).unwrap();
    assert!(matches!(wal.recover(), Err(LsmError::Codec(_))));
    assert_eq!(std::fs::read(dir.path().join("wal.log")).unwrap(), frame);
}

#[test]
fn mixed_plaintext_and_encrypted_replay_preserves_record_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let mut plain = LogRecord::new(b"plain".to_vec(), b"plain-value".to_vec());
    plain.timestamp = 10;
    let mut ttl = LogRecord::new(b"ttl".to_vec(), b"encrypted-value".to_vec());
    ttl.timestamp = 20;
    ttl.expires_at = Some(123456789);
    let mut deleted = LogRecord::tombstone(b"deleted".to_vec());
    deleted.timestamp = 30;
    let mut range = LogRecord::range_tombstone(b"a".to_vec(), b"z".to_vec());
    range.timestamp = 40;
    write(dir.path(), "default", &plaintext(), &plain);
    for record in [&ttl, &deleted, &range] {
        write(dir.path(), "default", &key(), record);
    }
    let wal = WriteAheadLog::new_with_encryption(dir.path(), "default", &key()).unwrap();
    let records = wal.recover().unwrap();
    assert_eq!(records, vec![plain, ttl, deleted, range]);
}

#[test]
fn authentication_failure_with_valid_crc_does_not_return_earlier_records() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "default",
        &plaintext(),
        &LogRecord::new(b"plain".to_vec(), b"value".to_vec()),
    );
    let prefix = std::fs::read(dir.path().join("wal.log")).unwrap();
    let record = LogRecord::new(b"secret".to_vec(), b"value".to_vec());
    let mut frame = encrypted_frame(&apexstore::infra::codec::encode(&record).unwrap());
    frame[17] ^= 0x80; // Change ciphertext after the version and 12-byte nonce.
    let checksum_offset = frame.len() - 4;
    let checksum = crc32fast::hash(&frame[..checksum_offset]);
    frame[checksum_offset..].copy_from_slice(&checksum.to_le_bytes());
    let original = [prefix, frame].concat();
    std::fs::write(dir.path().join("wal.log"), &original).unwrap();
    let wal = WriteAheadLog::new_with_encryption(dir.path(), "default", &key()).unwrap();
    assert!(matches!(
        wal.recover(),
        Err(LsmError::EncryptionAuthenticationFailed)
    ));
    assert_eq!(std::fs::read(dir.path().join("wal.log")).unwrap(), original);
}
