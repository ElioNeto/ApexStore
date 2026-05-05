use apexstore::infra::error::LsmError;
use apexstore::storage::builder::SstableBuilder;
use apexstore::storage::reader::SstableReader;
use apexstore::{LsmConfig, LsmEngine};
use tempfile::tempdir;

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};


#[test]
fn restart_recovers_from_wal() {
    let dir = tempdir().unwrap();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir.path().to_path_buf())
        .build()
        .unwrap();

    {
        let mut engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        engine.set("k1".to_string(), b"v1".to_vec()).unwrap();
    }

    let engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
    let v = engine.get("k1").unwrap().unwrap();
    assert_eq!(v, b"v1".to_vec());
}

#[test]
fn restart_after_flush_reads_sstable() {
    let dir = tempdir().unwrap();
    let cfg = LsmConfig::builder()
        // Minimum memtable size is 1024 bytes (1KB)
        .memtable_max_size(1024)
        .dir_path(dir.path().to_path_buf())
        .build()
        .unwrap();

    {
        let mut engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        // Write enough data to trigger flush (1KB memtable)
        // 50 entries * ~25 bytes (20 bytes value + key + overhead) = ~1250 bytes > 1024
        for i in 0..50 {
            engine.set(format!("k{i}"), vec![b'x'; 20]).unwrap();
        }
        // Force flush to ensure SSTable creation if automatic flush didn't happen
        // (though with 1KB limit it should happen automatically)
    }

    let engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
    let v = engine.get("k1").unwrap().unwrap();
    assert!(!v.is_empty());
}

#[test]
fn tombstone_persists_across_restart() {
    let dir = tempdir().unwrap();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir.path().to_path_buf())
        .build()
        .unwrap();

    {
        let mut engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        engine.set("k".to_string(), b"v".to_vec()).unwrap();
        engine.delete("k".to_string()).unwrap();
    }

    let engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
    assert!(engine.get("k").unwrap().is_none());
}

#[test]
fn wal_truncation_is_detected() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir_path.clone())
        .build()
        .unwrap();

    {
        let mut engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        engine.set("k1".to_string(), b"v1".to_vec()).unwrap();
    }

    let wal_path = dir_path.join("wal.log");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&wal_path)
        .unwrap();

    let len = file.metadata().unwrap().len();
    assert!(len > 1);

    file.set_len(len - 1).unwrap();

    let res = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096));
    match res {
        // Truncated WAL is detected as data corruption during recovery
        Err(LsmError::CorruptedData(_)) | Err(LsmError::WalCorruption) => {}
        Err(other) => panic!("expected corruption error, got: {other}"),
        Ok(_) => panic!("expected corruption error, got Ok"),
    }
}

#[test]
fn test_wal_partial_replay() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir_path.clone())
        .build()
        .unwrap();

    let wal_path = dir_path.join("wal.log");

    // Write N=5 records, recording WAL size after the first 4
    let size_after_4: u64;
    {
        let mut engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        for i in 0..4 {
            engine.set(format!("k{i}"), b"value".to_vec()).unwrap();
        }
        // Record WAL size after 4 records (each write_record fsyncs)
        size_after_4 = fs::metadata(&wal_path).map(|m| m.len()).unwrap();
        // Write the 5th record
        engine.set("k4".to_string(), b"value".to_vec()).unwrap();
    } // engine dropped (but not flushed, so WAL is source of truth)

    // Truncate WAL to size before the last record was written
    let file = OpenOptions::new().write(true).open(&wal_path).unwrap();
    file.set_len(size_after_4).unwrap();
    drop(file);

    // Reopen — should recover the first 4 records (N-1)
    let engine = LsmEngine::new_from_config(&cfg, apexstore::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();

    // Verify all N-1 records are present
    for i in 0..4 {
        let v = engine.get(format!("k{i}")).unwrap().expect(&format!("key k{i} should be recovered"));
        assert_eq!(v, b"value".to_vec());
    }

    // The 5th record should be lost (truncated)
    assert!(engine.get("k4").unwrap().is_none(), "key k4 should not be recovered after truncation");
}

#[test]
fn test_sstable_corruption() {
    use apexstore::infra::config::StorageConfig;
    use apexstore::core::log_record::LogRecord;
    use apexstore::storage::cache::GlobalBlockCache;

    let dir = tempdir().unwrap();
    let path = dir.path().join("test_corrupt.sst");
    let config = StorageConfig::default();

    // Build an SSTable with some data
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), timestamp).unwrap();

    for i in 0..10 {
        let key = format!("key_{:02}", i);
        let value = format!("value_{}", i);
        builder
            .add(key.as_bytes(), &LogRecord::new(key.as_bytes().to_vec(), value.as_bytes().to_vec()))
            .unwrap();
    }

    let sst_path = builder.finish().unwrap();

    // Open and verify we can read
    let reader = SstableReader::open(
        sst_path.clone(),
        config.clone(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    let result = reader.get(b"key_00").unwrap();
    assert!(result.is_some(), "Should find key_00 before corruption");

    // Corrupt the file by writing garbage after the magic number
    {
        let mut file = OpenOptions::new().write(true).open(&sst_path).unwrap();
        file.seek(SeekFrom::Start(8)).unwrap();
        let garbage = vec![0xFF; 20];
        file.write_all(&garbage).unwrap();
    }

    // Reopen and try to read — should get CorruptedData
    let reader2 = SstableReader::open(
        sst_path.clone(),
        config.clone(),
        GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    let result = reader2.get(b"key_00");
    match result {
        Err(LsmError::CorruptedData(msg)) => {
            assert!(
                msg.contains("CRC32 mismatch") || msg.contains("checksum"),
                "Expected CRC32 mismatch error, got: {}",
                msg
            );
        }
        other => panic!("Expected CorruptedData error, got: {:?}", other),
    }
}
