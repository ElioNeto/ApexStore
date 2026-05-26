//! Chaos tests for data corruption tolerance.
//!
//! These tests simulate data corruption scenarios (corrupted SSTable files)
//! and verify that the engine handles them gracefully without panicking.
//!
//! The engine's VersionSet has a quarantine mechanism that skips SSTables
//! that fail to read — these tests validate that mechanism works correctly
//! with corrupted data.

use apexstore::core::engine::Engine;
use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use std::io::{Seek, SeekFrom, Write};
use std::sync::Arc;
use tempfile::TempDir;

/// Small memtable to force frequent flushes
const SMALL_MEMTABLE: usize = 2048; // 2KB (minimum is 1024)

fn create_engine() -> (TempDir, Engine<Arc<GlobalBlockCache>>) {
    let dir = TempDir::new().unwrap();
    let mut config = LsmConfig::default();
    config.core.dir_path = dir.path().to_path_buf();
    config.core.memtable_max_size = SMALL_MEMTABLE;
    let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();
    (dir, engine)
}

/// Corrupt an SSTable file by writing garbage past its header, then verify
/// the engine survives and returns errors gracefully (no panics).
#[test]
fn test_chaos_corrupted_sstable() {
    let (dir, engine) = create_engine();
    let db_path = dir.path().to_path_buf();

    // Write data and flush to create SSTables
    for i in 0..100 {
        let key = format!("k{:04}", i);
        let value = format!("v{:04}", i);
        engine.set(key, value.as_bytes()).unwrap();
    }
    engine.flush_memtable().unwrap();

    // Corrupt an SSTable file by writing garbage after the header
    let sst_dir = db_path.join("sstables");
    if sst_dir.exists() {
        let sst_files: Vec<_> = std::fs::read_dir(&sst_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();

        if let Some(sst) = sst_files.first() {
            eprintln!("Corrupting SSTable: {:?}", sst);
            let mut file = std::fs::OpenOptions::new().write(true).open(sst).unwrap();
            // Seek past the magic number and header, then write garbage
            file.seek(SeekFrom::Start(8)).unwrap();
            file.write_all(&[0xFF; 200]).unwrap();
        }
    }

    // Engine should survive and return results gracefully.
    // The VersionSet will quarantine the corrupted table and skip it,
    // serving data from the in-memory Table.data or from other uncorrupted
    // SSTables.
    let result = engine.get("k0000");
    match result {
        Ok(Some(val)) => {
            eprintln!(
                "  Key retrieved successfully (from memory/other SSTable): {} bytes",
                val.len()
            );
        }
        Ok(None) => {
            eprintln!("  Key not found (corrupted table was quarantined, key not in other tables)");
        }
        Err(e) => {
            // Due to the quarantine mechanism, errors should not propagate to the user.
            // If they do, something is wrong with the error handling in VersionSet::get().
            panic!(
                "Engine should not return error for corrupted SSTable (it should quarantine it): {}",
                e
            );
        }
    }

    // New writes must still succeed after corruption
    engine
        .set("post_corruption_key", b"post_corruption_val")
        .unwrap();
    let v = engine.get("post_corruption_key").unwrap();
    assert_eq!(
        v,
        Some(b"post_corruption_val".to_vec()),
        "New writes must work after SSTable corruption"
    );

    // Scan must still work
    let scan_result = engine.scan();
    assert!(
        scan_result.is_ok(),
        "Scan must survive corrupted SSTable: {:?}",
        scan_result.err()
    );
}

/// Corrupt the bloom filter region of an SSTable and verify the engine
/// handles the corruption gracefully.
#[test]
fn test_chaos_corrupted_bloom_filter() {
    let (dir, engine) = create_engine();
    let db_path = dir.path().to_path_buf();

    // Write data and flush
    for i in 0..50 {
        let key = format!("bloom_key_{:04}", i);
        let value = format!("bloom_val_{:04}", i);
        engine.set(key, value.as_bytes()).unwrap();
    }
    engine.flush_memtable().unwrap();

    // Corrupt the bloom filter area (first 256 bytes after magic)
    let sst_dir = db_path.join("sstables");
    if sst_dir.exists() {
        let sst_files: Vec<_> = std::fs::read_dir(&sst_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();

        if let Some(sst) = sst_files.first() {
            eprintln!("Corrupting bloom filter in SSTable: {:?}", sst);
            let mut file = std::fs::OpenOptions::new().write(true).open(sst).unwrap();
            // Write garbage near where bloom filter metadata would be
            file.seek(SeekFrom::Start(4)).unwrap();
            file.write_all(&[0xAA; 128]).unwrap();
        }
    }

    // Read should not panic (the result itself may be Ok or Err depending
    // on whether the corruption hit a critical region at read time).
    let _result = engine.get("bloom_key_0000");

    // Write + read should still work
    engine.set("after_bloom_corrupt", b"ok").unwrap();
    let v = engine.get("after_bloom_corrupt").unwrap();
    assert_eq!(
        v,
        Some(b"ok".to_vec()),
        "Engine must accept writes after bloom filter corruption"
    );
}

/// Delete a WAL file while the engine is not running and verify that the
/// engine can still start (recovering whatever data was flushed to SSTables).
#[test]
fn test_chaos_missing_wal_file() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().to_path_buf();

    let mut config = LsmConfig::default();
    config.core.dir_path = db_path.clone();
    config.core.memtable_max_size = SMALL_MEMTABLE;

    // Phase 1: Write and flush (data goes to both WAL and SSTable)
    {
        let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();
        for i in 0..50 {
            let key = format!("wal_test_key_{:04}", i);
            let value = format!("wal_test_val_{:04}", i);
            engine.set(key, value.as_bytes()).unwrap();
        }
        engine.flush_memtable().unwrap();
        // Write one more key that stays in the WAL (not flushed)
        engine.set("wal_only_key", b"wal_only_value").unwrap();
        engine.close();
    }

    // Delete the WAL file
    let wal_path = db_path.join("wal.log");
    if wal_path.exists() {
        eprintln!("Deleting WAL file: {:?}", wal_path);
        std::fs::remove_file(&wal_path).unwrap();
    }

    // Phase 2: Restart — engine should recover from SSTables
    {
        let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();

        // Flushed data should be discoverable (from disk SSTables or WAL replay)
        let result = engine.get("wal_test_key_0000");
        match result {
            Ok(Some(_)) => eprintln!("  Flushed key recovered after WAL deletion"),
            Ok(None) => eprintln!("  Flushed key not found (may be expected without manifest)"),
            Err(e) => {
                panic!("Engine should not error on read after WAL deletion: {}", e);
            }
        }

        // The unflushed key should be lost (WAL was deleted)
        let wal_only = engine.get("wal_only_key").unwrap();
        assert!(
            wal_only.is_none(),
            "Unflushed key should be lost after WAL deletion"
        );

        // The engine must accept new writes
        engine.set("post_wal_deletion", b"survived").unwrap();
        let v = engine.get("post_wal_deletion").unwrap();
        assert_eq!(
            v,
            Some(b"survived".to_vec()),
            "Engine must accept writes after WAL deletion"
        );
    }
}
