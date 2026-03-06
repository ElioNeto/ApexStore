use apexstore::core::log_record::LogRecord;
use apexstore::infra::config::StorageConfig;
use apexstore::infra::error::Result;
use apexstore::storage::builder::SstableBuilder;
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::storage::reader::SstableReader;
use std::sync::Arc;
use tempfile::tempdir;

fn create_test_record(key: &str, value: &[u8]) -> LogRecord {
    LogRecord::new(key.to_string(), value.to_vec())
}

fn create_test_cache(config: &StorageConfig) -> Arc<GlobalBlockCache> {
    GlobalBlockCache::new(config.block_cache_size_mb, config.block_size)
}

#[test]
fn test_sstable_v2_roundtrip_small() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("roundtrip_small.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write 10 records
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 123)?;
    let test_data: Vec<_> = (0..10)
        .map(|i| (format!("key_{:04}", i), format!("value_{:04}", i)))
        .collect();

    for (key, value) in &test_data {
        builder.add(key.as_bytes(), &create_test_record(key, value.as_bytes()))?;
    }
    builder.finish()?;

    // Read and verify
    let mut reader = SstableReader::open(path, config, cache)?;

    for (key, expected_value) in &test_data {
        let record = reader.get(key)?.expect("Key should exist");
        assert_eq!(record.value, expected_value.as_bytes());
    }

    // Verify non-existent keys
    assert!(reader.get("missing_key")?.is_none());

    Ok(())
}

#[test]
fn test_sstable_v2_roundtrip_large() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("roundtrip_large.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write 1000 records
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 456)?;
    let test_data: Vec<_> = (0..1000)
        .map(|i| (format!("key_{:06}", i), format!("value_{:06}", i)))
        .collect();

    for (key, value) in &test_data {
        builder.add(key.as_bytes(), &create_test_record(key, value.as_bytes()))?;
    }
    builder.finish()?;

    // Read and verify all records
    let mut reader = SstableReader::open(path, config, cache)?;

    for (key, expected_value) in &test_data {
        let record = reader.get(key)?.expect("Key should exist");
        assert_eq!(record.value, expected_value.as_bytes());
    }

    Ok(())
}

#[test]
fn test_sstable_v2_multiple_blocks() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("multi_block.sst");
    let mut config = StorageConfig::default();
    config.block_size = 512; // Small blocks to force multiple blocks
    let cache = create_test_cache(&config);

    // Write enough data to span multiple blocks
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 789)?;
    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = vec![b'x'; 50]; // 50 bytes per value
        builder.add(key.as_bytes(), &create_test_record(&key, &value))?;
    }
    builder.finish()?;

    // Read and verify
    let mut reader = SstableReader::open(path, config, cache)?;

    // Verify metadata shows multiple blocks
    assert!(
        reader.metadata().blocks.len() > 1,
        "Should have multiple blocks"
    );

    // Verify all records are readable
    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let record = reader.get(&key)?;
        assert!(record.is_some(), "Key {} should exist", key);
    }

    Ok(())
}

#[test]
fn test_sstable_v2_bloom_filter_effectiveness() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("bloom_test.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write 500 records
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 999)?;
    for i in 0..500 {
        let key = format!("existing_key_{:04}", i);
        builder.add(key.as_bytes(), &create_test_record(&key, b"value"))?;
    }
    builder.finish()?;

    // Test Bloom filter
    let reader = SstableReader::open(path, config, cache)?;

    // All existing keys should pass Bloom filter
    for i in 0..500 {
        let key = format!("existing_key_{:04}", i);
        assert!(
            reader.might_contain(&key),
            "Existing key should pass Bloom filter"
        );
    }

    // Count false positives for non-existent keys
    let false_positives = (1000..1500)
        .filter(|i| reader.might_contain(&format!("nonexistent_{}", i)))
        .count();

    // With 1% FP rate and 500 checks, expect < 10 false positives
    assert!(
        false_positives < 10,
        "Too many false positives: {}",
        false_positives
    );

    Ok(())
}

#[test]
fn test_sstable_v2_boundary_keys() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("boundary.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write records with boundary keys
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 111)?;
    builder.add(b"aaa", &create_test_record("aaa", b"first"))?;
    builder.add(b"mmm", &create_test_record("mmm", b"middle"))?;
    builder.add(b"zzz", &create_test_record("zzz", b"last"))?;
    builder.finish()?;

    let mut reader = SstableReader::open(path, config, cache)?;

    // Test exact boundary keys
    assert!(reader.get("aaa")?.is_some(), "First key should exist");
    assert!(reader.get("zzz")?.is_some(), "Last key should exist");

    // Test keys before first
    assert!(
        reader.get("000")?.is_none(),
        "Key before first should not exist"
    );
    assert!(
        reader.get("aa")?.is_none(),
        "Key before first should not exist"
    );

    // Test keys after last
    assert!(
        reader.get("zzzz")?.is_none(),
        "Key after last should not exist"
    );

    // Test keys between boundaries
    assert!(
        reader.get("bbb")?.is_none(),
        "Non-existent key should not exist"
    );
    assert!(reader.get("mmm")?.is_some(), "Middle key should exist");

    Ok(())
}

#[test]
fn test_sstable_v2_scan() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("scan_test.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write ordered records
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 222)?;
    let test_keys = vec!["apple", "banana", "cherry", "date", "elderberry"];

    for key in &test_keys {
        builder.add(
            key.as_bytes(),
            &create_test_record(key, format!("{}_value", key).as_bytes()),
        )?;
    }
    builder.finish()?;

    // Scan all records
    let mut reader = SstableReader::open(path, config, cache)?;
    let records = reader.scan()?;

    assert_eq!(records.len(), test_keys.len(), "Should scan all records");

    // Verify order is preserved
    for (i, key) in test_keys.iter().enumerate() {
        assert_eq!(
            records[i].0,
            key.as_bytes(),
            "Key order should be preserved"
        );
    }

    Ok(())
}

#[test]
fn test_sstable_v2_large_values() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("large_values.sst");
    let mut config = StorageConfig::default();
    // Increase block size to accommodate large values
    config.block_size = 16384; // 16KB blocks
    let cache = create_test_cache(&config);

    // Write records with large values (but smaller than block size)
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 333)?;
    let large_value = vec![b'x'; 8000]; // 8KB value (fits in 16KB block)

    for i in 0..10 {
        let key = format!("key_{}", i);
        builder.add(key.as_bytes(), &create_test_record(&key, &large_value))?;
    }
    builder.finish()?;

    // Read and verify
    let mut reader = SstableReader::open(path, config, cache)?;

    for i in 0..10 {
        let key = format!("key_{}", i);
        let record = reader.get(&key)?.expect("Key should exist");
        assert_eq!(record.value.len(), 8000, "Value size should be 8KB");
        assert_eq!(record.value, large_value, "Value content should match");
    }

    Ok(())
}

#[test]
fn test_sstable_v2_cache_effectiveness() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("cache_test.sst");
    let mut config = StorageConfig::default();
    config.block_cache_size_mb = 10; // Small cache
    config.block_size = 512;
    let cache = create_test_cache(&config);

    // Write multiple blocks
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 444)?;
    for i in 0..50 {
        let key = format!("key_{:03}", i);
        let value = vec![b'x'; 30];
        builder.add(key.as_bytes(), &create_test_record(&key, &value))?;
    }
    builder.finish()?;

    let mut reader = SstableReader::open(path, config, cache)?;

    // Read same keys multiple times (should benefit from cache)
    for _ in 0..3 {
        for i in 0..50 {
            let key = format!("key_{:03}", i);
            let record = reader.get(&key)?;
            assert!(record.is_some(), "Key should exist");
        }
    }

    Ok(())
}

#[test]
fn test_sstable_v2_empty_key() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("empty_key.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write with empty string key
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 555)?;
    builder.add(b"", &create_test_record("", b"empty_key_value"))?;
    builder.add(
        b"normal_key",
        &create_test_record("normal_key", b"normal_value"),
    )?;
    builder.finish()?;

    let mut reader = SstableReader::open(path, config, cache)?;

    // Should be able to read empty key
    let record = reader.get("")?.expect("Empty key should exist");
    assert_eq!(record.value, b"empty_key_value");

    // Normal key should also work
    let record = reader.get("normal_key")?.expect("Normal key should exist");
    assert_eq!(record.value, b"normal_value");

    Ok(())
}

#[test]
fn test_sstable_v2_unicode_keys() -> Result<()> {
    let dir = tempdir()?;
    let path = dir.path().join("unicode.sst");
    let config = StorageConfig::default();
    let cache = create_test_cache(&config);

    // Write with unicode keys (pre-sorted by UTF-8 byte order)
    let mut builder = SstableBuilder::new(path.clone(), config.clone(), 666)?;
    // Keys must be sorted by UTF-8 byte order for SSTable
    let mut unicode_keys = vec!["hello", "こんにちは", "你好", "مرحبا", "привет"];
    unicode_keys.sort();

    for key in &unicode_keys {
        builder.add(
            key.as_bytes(),
            &create_test_record(key, format!("{}_value", key).as_bytes()),
        )?;
    }
    builder.finish()?;

    let mut reader = SstableReader::open(path, config, cache)?;

    // Verify all unicode keys are readable
    for key in &unicode_keys {
        let record = reader.get(key)?;
        assert!(record.is_some(), "Unicode key '{}' should exist", key);
        if let Some(r) = record {
            let expected = format!("{}_value", key);
            assert_eq!(
                r.value,
                expected.as_bytes(),
                "Value for '{}' should match",
                key
            );
        }
    }

    Ok(())
}
