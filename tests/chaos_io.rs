//! Chaos tests for I/O fault tolerance.
//!
//! These tests simulate I/O failures (missing/deleted SSTable files) and verify
//! that the engine survives and continues to operate without panicking or
//! corrupting its data structures.
//!
//! The engine already has a quarantine mechanism in VersionSet that skips
//! SSTables that fail to read — these tests validate that mechanism end-to-end.

use apexstore::core::engine::Engine;
use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use std::sync::Arc;
use tempfile::TempDir;

/// Small memtable to force frequent flushes
const SMALL_MEMTABLE: usize = 2048; // 2KB (minimum is 1024)

/// Helper: create an engine with a known directory path.
fn create_engine() -> (TempDir, Engine<Arc<GlobalBlockCache>>) {
    let dir = TempDir::new().unwrap();
    let mut config = LsmConfig::default();
    config.core.dir_path = dir.path().to_path_buf();
    config.core.memtable_max_size = SMALL_MEMTABLE;
    let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();
    (dir, engine)
}

/// Write enough data to force a flush into SSTables, then delete one of the
/// SSTable files and verify the engine survives.
#[test]
fn test_chaos_sstable_deleted_after_flush() {
    let (dir, engine) = create_engine();
    let db_path = dir.path().to_path_buf();

    // Write enough data to trigger flushes (SMALL_MEMTABLE = 2KB, and
    // write_buffer_limit = memtable_max_size * 4 = 8KB)
    for i in 0..500 {
        let key = format!("key_{:04}", i);
        let value = format!("value_{:04}", i);
        engine.set(key, value.as_bytes()).unwrap();
    }

    // Force flush all pending memtables to SSTables
    engine.flush_memtable().unwrap();

    // Verify we have data before deleting anything
    let result = engine.get("key_0000").unwrap();
    assert!(result.is_some(), "Data should be present before deletion");

    // Locate and delete one SSTable file
    let sst_dir = db_path.join("sstables");
    if sst_dir.exists() {
        let sst_files: Vec<_> = std::fs::read_dir(&sst_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();

        if let Some(first_sst) = sst_files.first() {
            eprintln!("Deleting SSTable: {:?}", first_sst);
            std::fs::remove_file(first_sst).unwrap();
        }
    }

    // Engine should still be able to read surviving data without panicking.
    // Some keys may still be served from the in-memory Table.data (which
    // survives across flushes), or from other uncorrupted SSTables.
    let result = engine.get("key_0000");
    assert!(
        result.is_ok(),
        "Engine should survive missing SSTable file: {:?}",
        result.err()
    );

    // Verify we can still write new data after the file deletion
    engine.set("new_key_after_deletion", b"new_value").unwrap();
    let readback = engine.get("new_key_after_deletion").unwrap();
    assert_eq!(
        readback,
        Some(b"new_value".to_vec()),
        "Engine should accept new writes after SSTable deletion"
    );

    // Verify scan still works
    let scan_result = engine.scan();
    assert!(
        scan_result.is_ok(),
        "Engine scan should survive missing SSTable file: {:?}",
        scan_result.err()
    );
}

/// Delete an SSTable file *while* compaction might reference it and verify
/// the engine doesn't crash.
#[test]
fn test_chaos_compact_with_missing_sstable() {
    let (dir, engine) = create_engine();
    let db_path = dir.path().to_path_buf();

    // Write data to create multiple SSTables
    for batch in 0..10 {
        for i in 0..100 {
            let key = format!("batch{}_key_{:04}", batch, i);
            let value = format!("batch{}_val_{:04}", batch, i);
            engine.set(key, value.as_bytes()).unwrap();
        }
        // Flush each batch to create an SSTable
        engine.flush_memtable().unwrap();
    }

    // Delete one SSTable file
    let sst_dir = db_path.join("sstables");
    if sst_dir.exists() {
        let sst_files: Vec<_> = std::fs::read_dir(&sst_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();

        if let Some(sst) = sst_files.first() {
            eprintln!("Deleting SSTable for compaction test: {:?}", sst);
            std::fs::remove_file(sst).unwrap();
        }
    }

    // Run compaction — should not panic even if one file is missing
    let compact_result = engine.compact();
    match compact_result {
        Ok(_) => eprintln!("Compaction succeeded despite missing file"),
        Err(e) => eprintln!(
            "Compaction returned error (expected with missing file): {}",
            e
        ),
    }

    // Engine should still be operational after compaction attempt
    let result = engine.get("batch0_key_0000");
    assert!(
        result.is_ok(),
        "Engine should survive compaction with missing SSTable: {:?}",
        result.err()
    );
}

/// Restart the engine after deleting an SSTable file — the engine should
/// recover from the WAL and/or survive the missing files on next open.
#[test]
fn test_chaos_restart_with_missing_sstable() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().to_path_buf();

    let mut config = LsmConfig::default();
    config.core.dir_path = db_path.clone();
    config.core.memtable_max_size = SMALL_MEMTABLE;

    // Phase 1: Create engine, write data, flush, delete an SSTable
    {
        let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();

        for i in 0..200 {
            let key = format!("restart_key_{:04}", i);
            let value = format!("restart_val_{:04}", i);
            engine.set(key, value.as_bytes()).unwrap();
        }
        engine.flush_memtable().unwrap();

        // Delete the first SSTable file
        let sst_dir = db_path.join("sstables");
        let sst_files: Vec<_> = std::fs::read_dir(&sst_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();

        if let Some(sst) = sst_files.first() {
            eprintln!("Deleting SSTable before restart: {:?}", sst);
            std::fs::remove_file(sst).unwrap();
        }

        // Close the engine cleanly
        engine.close();
    }

    // Phase 2: Restart — the engine should discover surviving SSTables
    // and recover any unflushed data from the WAL.
    {
        let engine = Engine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap();

        // The engine must start without panic
        let stats = engine.stats("default");
        assert!(
            stats.is_ok(),
            "Engine stats should work after restart with missing SSTable"
        );

        // Verify we can still read and write
        let result = engine.get("restart_key_0000");
        match result {
            Ok(Some(val)) => {
                eprintln!("  Key recovered after restart: {} bytes", val.len());
            }
            Ok(None) => {
                eprintln!("  Key not found after restart (may be in deleted file)");
            }
            Err(e) => {
                panic!(
                    "Engine should not error on read after restart with missing SSTable: {}",
                    e
                );
            }
        }

        // New writes must still work
        engine.set("post_restart_key", b"post_restart_val").unwrap();
        let v = engine.get("post_restart_key").unwrap();
        assert_eq!(
            v,
            Some(b"post_restart_val".to_vec()),
            "New writes must work after restart with missing SSTable"
        );
    }
}
