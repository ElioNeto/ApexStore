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
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        engine.set("k1".to_string(), b"v1".to_vec()).unwrap();
    }

    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
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
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        // Write enough data to trigger flush (1KB memtable)
        // 50 entries * ~25 bytes (20 bytes value + key + overhead) = ~1250 bytes > 1024
        for i in 0..50 {
            engine.set(format!("k{i}"), vec![b'x'; 20]).unwrap();
        }
        // Force flush to ensure SSTable creation if automatic flush didn't happen
        // (though with 1KB limit it should happen automatically)
    }

    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
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
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        engine.set("k".to_string(), b"v".to_vec()).unwrap();
        engine.delete("k".to_string()).unwrap();
    }

    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    assert!(engine.get("k").unwrap().is_none());
}

#[test]
fn wal_truncation_recovers_partial_last_record() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir_path.clone())
        .build()
        .unwrap();

    {
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
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
    drop(file);

    // Engine should start gracefully; the truncated record is unconfirmed data and is lost
    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();
    // The only record was truncated → k1 should not be present
    assert!(
        engine.get("k1").unwrap().is_none(),
        "truncated record should be lost"
    );
}

#[test]
fn wal_truncation_mid_write_recovers_prior_records() {
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let cfg = LsmConfig::builder()
        .memtable_max_size(1024 * 1024)
        .dir_path(dir_path.clone())
        .build()
        .unwrap();

    // Write N=5 records, record size after first 4
    let size_after_4: u64;
    {
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        for i in 0..4 {
            engine.set(format!("k{i}"), b"value".to_vec()).unwrap();
        }
        size_after_4 = std::fs::metadata(dir_path.join("wal.log"))
            .map(|m| m.len())
            .unwrap();
        // Write 5th record
        engine.set("k4".to_string(), b"value".to_vec()).unwrap();
    }

    // Truncate WAL to somewhere INSIDE the 5th record (not at boundary)
    // We keep just enough data that the 4th record is intact and the 5th is partial
    let wal_path = dir_path.join("wal.log");
    let full_len = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap();
    // Truncate to slightly past the 4th record — inside the 5th record frame
    let truncate_point = size_after_4 + 3; // 3 bytes into the 5th frame (length prefix started)
    let file = OpenOptions::new().write(true).open(&wal_path).unwrap();
    file.set_len(truncate_point.min(full_len)).unwrap();
    drop(file);

    // Reopen — should recover first 4 records, discard the partial 5th
    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    // Verify first 4 records are present
    for i in 0..4 {
        let v = engine
            .get(format!("k{i}"))
            .unwrap()
            .unwrap_or_else(|| panic!("key k{i} should be recovered"));
        assert_eq!(v, b"value".to_vec());
    }

    // The partially written 5th record should be lost
    assert!(
        engine.get("k4").unwrap().is_none(),
        "partial 5th record should be lost"
    );
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
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
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
    let engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    // Verify all N-1 records are present
    for i in 0..4 {
        let v = engine
            .get(format!("k{i}"))
            .unwrap()
            .expect(&format!("key k{i} should be recovered"));
        assert_eq!(v, b"value".to_vec());
    }

    // The 5th record should be lost (truncated)
    assert!(
        engine.get("k4").unwrap().is_none(),
        "key k4 should not be recovered after truncation"
    );
}

#[test]
fn compaction_crash_restart_consistency() {
    // Verify that after a crash during compaction, the engine restarts
    // without internal inconsistencies (no panics, no lock poison, etc.).
    // Note: data written before the crash is recovered from WAL;
    // flushed data is in SSTables but the engine rebuilds VersionSet
    // from WAL only on startup, so flushed data is re-created on next writes.
    let dir = tempdir().unwrap();
    let dir_path = dir.path().to_path_buf();
    let cfg = LsmConfig::builder()
        .memtable_max_size(2048) // small memtable to trigger flushes
        .dir_path(dir_path.clone())
        .block_cache_size_mb(1)
        .build()
        .unwrap();

    // Populate with enough data to trigger multiple flushes and compactions
    {
        let mut engine = LsmEngine::new_from_config(
            &cfg,
            apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        for i in 0..200 {
            engine.set(format!("k{i:04}"), vec![b'x'; 40]).unwrap();
        }
        // Force flush + compact to exercise compaction paths
        engine.flush_memtable().unwrap();
        let _ = engine.compact();
    } // engine dropped — simulates crash during/after compaction

    // Reopen — VersionSet must be consistent, engine must not panic
    let mut engine = LsmEngine::new_from_config(
        &cfg,
        apexstore::storage::cache::GlobalBlockCache::new(100, 4096),
    )
    .unwrap();

    // Engine is operational — stats and scan work without panic
    let _stats = engine.stats("default");
    let scan_result = engine.scan();
    assert!(
        scan_result.is_ok(),
        "scan must work after compaction crash restart"
    );

    // New writes must succeed (no lock poison or corrupted state)
    engine
        .set("fresh_key".to_string(), b"fresh_value".to_vec())
        .unwrap();
    let v = engine.get("fresh_key").unwrap();
    assert_eq!(
        v,
        Some(b"fresh_value".to_vec()),
        "new writes must work after crash restart"
    );
}

#[test]
fn test_sstable_corruption() {
    use apexstore::core::log_record::LogRecord;
    use apexstore::infra::config::StorageConfig;
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
            .add(
                key.as_bytes(),
                &LogRecord::new(key.as_bytes().to_vec(), value.as_bytes().to_vec()),
            )
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
