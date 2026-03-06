use crate::core::log_record::LogRecord;
use crate::infra::codec::decode;
use crate::infra::config::StorageConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::block::Block;
use crate::storage::builder::{BlockMeta, MetaBlock};
use crate::storage::cache::{CacheKey, GlobalBlockCache};
use bloomfilter::Bloom;
use lz4_flex::decompress_size_prepended;
use parking_lot::Mutex;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

const SST_MAGIC_V2: &[u8; 8] = b"LSMSST03";
const FOOTER_SIZE: u64 = 8;

/// SSTable V2 Reader with sparse index, Bloom filter, and shared global block caching
///
/// # Thread Safety
///
/// This reader is designed for concurrent access. Multiple threads can safely call
/// `get()` and `scan()` methods simultaneously. Internal synchronization is provided by:
/// - `Mutex<File>` for thread-safe file operations  
/// - `GlobalBlockCache` (has internal Mutex) for thread-safe cache access
/// - Immutable `metadata` and `bloom_filter` (no synchronization needed)
///
/// # Performance
///
/// Lock contention is minimized by:
/// - Bloom filter checks are lock-free (immutable data)
/// - Binary search on metadata is lock-free (immutable data)
/// - File and cache locks are held only during I/O operations
/// - Block decompression happens outside of locks
#[derive(Debug)]
pub struct SstableReader {
    metadata: MetaBlock,
    bloom_filter: Bloom<[u8]>,
    file: Mutex<File>,
    block_cache: Arc<GlobalBlockCache>,
    path: PathBuf,
    #[allow(dead_code)]
    config: StorageConfig,
}

impl SstableReader {
    /// Open an SSTable V2 file for reading with a shared block cache
    ///
    /// # Arguments
    /// * `path` - Path to the SSTable file
    /// * `config` - Storage configuration
    /// * `block_cache` - Shared global block cache
    pub fn open(
        path: PathBuf,
        config: StorageConfig,
        block_cache: Arc<GlobalBlockCache>,
    ) -> Result<Self> {
        let mut file = File::open(&path)?;

        // Verify magic number
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != SST_MAGIC_V2 {
            return Err(LsmError::InvalidSstableFormat(format!(
                "Invalid magic number: expected {:?}, found {:?}",
                SST_MAGIC_V2, magic
            )));
        }

        // Read footer to get metadata offset
        let meta_offset = Self::read_footer(&mut file)?;

        // Read and decompress metadata block
        let metadata = Self::read_meta_block(&mut file, meta_offset)?;

        // Deserialize Bloom filter from stored bytes (clone to avoid moving)
        let bloom_filter =
            Bloom::<[u8]>::from_bytes(metadata.bloom_filter_data.clone()).map_err(|e| {
                LsmError::CompactionFailed(format!("Bloom filter deserialization failed: {}", e))
            })?;

        Ok(Self {
            metadata,
            bloom_filter,
            file: Mutex::new(file),
            block_cache,
            path,
            config,
        })
    }

    /// Check if key might exist using Bloom filter (fast pre-check)
    ///
    /// This method is lock-free and very fast. It should be called before `get()`
    /// to avoid unnecessary I/O for keys that definitely don't exist.
    pub fn might_contain(&self, key: &str) -> bool {
        self.bloom_filter.check(key.as_bytes())
    }

    /// Retrieve a value by key using sparse index and Bloom filter
    ///
    /// # Thread Safety
    /// This method can be safely called concurrently from multiple threads.
    /// Locks are held only during cache access and file I/O.
    pub fn get(&self, key: &str) -> Result<Option<LogRecord>> {
        // Fast rejection using Bloom filter (no lock needed)
        if !self.might_contain(key) {
            return Ok(None);
        }

        // Binary search on sparse index to find the block (no lock needed - immutable)
        let block_meta = match self.binary_search_block(key.as_bytes()) {
            Some(meta) => meta.clone(),
            None => return Ok(None),
        };

        // Read and decompress the block (with caching)
        let block_data = self.read_block(&block_meta)?;

        // Deserialize block (no lock needed)
        let block = Block::decode(&block_data);

        // Linear scan within the block to find the key (no lock needed)
        Self::search_in_block(&block, key.as_bytes())
    }

    /// Search for a key within a decoded block
    fn search_in_block(block: &Block, key: &[u8]) -> Result<Option<LogRecord>> {
        // Access block data through pub(crate) fields
        for &offset in &block.offsets {
            let offset = offset as usize;
            if offset + 2 > block.data.len() {
                break;
            }

            // Read key length
            let key_len = u16::from_le_bytes([block.data[offset], block.data[offset + 1]]) as usize;
            if offset + 2 + key_len + 2 > block.data.len() {
                break;
            }

            // Read key
            let entry_key = &block.data[offset + 2..offset + 2 + key_len];

            if entry_key == key {
                // Read value length
                let val_len_offset = offset + 2 + key_len;
                let val_len = u16::from_le_bytes([
                    block.data[val_len_offset],
                    block.data[val_len_offset + 1],
                ]) as usize;

                if val_len_offset + 2 + val_len > block.data.len() {
                    break;
                }

                // Read value
                let entry_value = &block.data[val_len_offset + 2..val_len_offset + 2 + val_len];

                // Decode the LogRecord from value
                let record: LogRecord = decode(entry_value)?;
                return Ok(Some(record));
            }
        }

        Ok(None)
    }

    /// Scan all records in the SSTable (for compaction)
    ///
    /// # Thread Safety
    /// This method can be safely called concurrently from multiple threads.
    pub fn scan(&self) -> Result<Vec<(Vec<u8>, LogRecord)>> {
        let mut records = Vec::new();

        // Clone blocks to avoid borrow issues (immutable, no lock needed)
        let blocks = self.metadata.blocks.clone();

        for block_meta in &blocks {
            let block_data = self.read_block(block_meta)?;
            let block = Block::decode(&block_data);

            // Access block data through pub(crate) fields
            for &offset in &block.offsets {
                let offset = offset as usize;
                if offset + 2 > block.data.len() {
                    break;
                }

                // Read key length
                let key_len =
                    u16::from_le_bytes([block.data[offset], block.data[offset + 1]]) as usize;
                if offset + 2 + key_len + 2 > block.data.len() {
                    break;
                }

                // Read key
                let key = block.data[offset + 2..offset + 2 + key_len].to_vec();

                // Read value length
                let val_len_offset = offset + 2 + key_len;
                let val_len = u16::from_le_bytes([
                    block.data[val_len_offset],
                    block.data[val_len_offset + 1],
                ]) as usize;

                if val_len_offset + 2 + val_len > block.data.len() {
                    break;
                }

                // Read value
                let value = &block.data[val_len_offset + 2..val_len_offset + 2 + val_len];

                // Decode the LogRecord from value
                let record: LogRecord = decode(value)?;
                records.push((key, record));
            }
        }

        Ok(records)
    }

    /// Get metadata information
    pub fn metadata(&self) -> &MetaBlock {
        &self.metadata
    }

    /// Get file path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    // Private helper methods

    fn read_footer(file: &mut File) -> Result<u64> {
        // Seek to the last 8 bytes (footer)
        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;

        let mut footer_bytes = [0u8; 8];
        file.read_exact(&mut footer_bytes)?;

        let meta_offset = u64::from_le_bytes(footer_bytes);
        Ok(meta_offset)
    }

    fn read_meta_block(file: &mut File, offset: u64) -> Result<MetaBlock> {
        // Seek to metadata block
        file.seek(SeekFrom::Start(offset))?;

        // Read compressed metadata until footer
        let file_len = file.metadata()?.len();
        let meta_size = (file_len - offset - FOOTER_SIZE) as usize;

        let mut compressed_meta = vec![0u8; meta_size];
        file.read_exact(&mut compressed_meta)?;

        // Decompress metadata
        let decompressed = decompress_size_prepended(&compressed_meta).map_err(|e| {
            LsmError::DecompressionFailed(format!("Metadata decompression failed: {}", e))
        })?;

        // Deserialize metadata
        let metadata: MetaBlock = decode(&decompressed)?;
        Ok(metadata)
    }

    fn read_block(&self, block_meta: &BlockMeta) -> Result<Vec<u8>> {
        // Create cache key with file path and block offset
        let cache_key = CacheKey::new(&self.path, block_meta.offset);

        // Check shared cache first (GlobalBlockCache has internal Mutex)
        if let Some(cached) = self.block_cache.get(&cache_key) {
            return Ok((*cached).clone());
        }

        // Cache miss - read from disk (lock released during decompression)
        let block_data = self.read_and_decompress_block(block_meta)?;

        // Store in shared cache (GlobalBlockCache has internal Mutex)
        self.block_cache.put(cache_key, block_data.clone());

        Ok(block_data)
    }

    fn read_and_decompress_block(&self, block_meta: &BlockMeta) -> Result<Vec<u8>> {
        // Read compressed block (lock held only during I/O)
        let compressed_block = {
            let mut file = self.file.lock();
            file.seek(SeekFrom::Start(block_meta.offset))?;
            let mut compressed_block = vec![0u8; block_meta.size as usize];
            file.read_exact(&mut compressed_block)?;
            compressed_block
        };

        // Decompress block (no lock - CPU intensive work)
        let decompressed = decompress_size_prepended(&compressed_block).map_err(|e| {
            LsmError::DecompressionFailed(format!(
                "Block decompression failed at offset {}: {}",
                block_meta.offset, e
            ))
        })?;

        // Verify decompressed size matches metadata
        if decompressed.len() != block_meta.uncompressed_size as usize {
            return Err(LsmError::CorruptedData(format!(
                "Block size mismatch: expected {}, got {}",
                block_meta.uncompressed_size,
                decompressed.len()
            )));
        }

        Ok(decompressed)
    }

    fn binary_search_block(&self, key: &[u8]) -> Option<&BlockMeta> {
        // If key is smaller than the first key in the SSTable, it doesn't exist
        if key < self.metadata.min_key.as_slice() {
            return None;
        }

        // If key is larger than the last key in the SSTable, it doesn't exist
        if key > self.metadata.max_key.as_slice() {
            return None;
        }

        // Binary search using partition_point to find the block where first_key <= search_key
        let idx = self
            .metadata
            .blocks
            .partition_point(|block_meta| block_meta.first_key.as_slice() <= key);

        // If idx is 0, key is smaller than all first_keys
        if idx == 0 {
            return None;
        }

        // Return the block at idx - 1 (the last block where first_key <= search_key)
        Some(&self.metadata.blocks[idx - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::builder::SstableBuilder;
    use std::thread;
    use tempfile::tempdir;

    fn create_test_record(key: &str, value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_string(), value.to_vec())
    }

    fn create_test_cache(config: &StorageConfig) -> Arc<GlobalBlockCache> {
        GlobalBlockCache::new(config.block_cache_size_mb, config.block_size)
    }

    #[test]
    fn test_reader_basic_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write SSTable
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 123).unwrap();
        builder
            .add(b"key1", &create_test_record("key1", b"value1"))
            .unwrap();
        builder
            .add(b"key2", &create_test_record("key2", b"value2"))
            .unwrap();
        builder
            .add(b"key3", &create_test_record("key3", b"value3"))
            .unwrap();
        builder.finish().unwrap();

        // Read SSTable
        let reader = SstableReader::open(path, config, cache).unwrap();

        // Verify reads (note: now uses &self, not &mut self)
        let record1 = reader.get("key1").unwrap().unwrap();
        assert_eq!(record1.value, b"value1");

        let record2 = reader.get("key2").unwrap().unwrap();
        assert_eq!(record2.value, b"value2");

        let record3 = reader.get("key3").unwrap().unwrap();
        assert_eq!(record3.value, b"value3");

        // Verify non-existent key
        assert!(reader.get("key4").unwrap().is_none());
    }

    #[test]
    fn test_reader_bloom_filter() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bloom_test.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write SSTable with known keys
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 456).unwrap();
        for i in 0..100 {
            let key = format!("key_{:03}", i);
            builder
                .add(key.as_bytes(), &create_test_record(&key, b"value"))
                .unwrap();
        }
        builder.finish().unwrap();

        // Read and test Bloom filter
        let reader = SstableReader::open(path, config, cache).unwrap();

        // Keys that exist should pass Bloom filter
        assert!(reader.might_contain("key_000"));
        assert!(reader.might_contain("key_050"));
        assert!(reader.might_contain("key_099"));

        // Non-existent keys might have false positives, but should mostly return false
        let false_positive_count = (1000..1100)
            .filter(|i| reader.might_contain(&format!("nonexistent_{}", i)))
            .count();

        // With 1% FP rate and 100 checks, expect < 5 false positives
        assert!(
            false_positive_count < 5,
            "Too many false positives: {}",
            false_positive_count
        );
    }

    #[test]
    fn test_reader_multiple_blocks() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("multi_block.sst");
        let mut config = StorageConfig::default();
        config.block_size = 256; // Small blocks to force multiple blocks
        let cache = create_test_cache(&config);

        // Write many records to span multiple blocks
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 789).unwrap();
        for i in 0..50 {
            let key = format!("key_{:03}", i);
            let value = vec![b'x'; 20];
            builder
                .add(key.as_bytes(), &create_test_record(&key, &value))
                .unwrap();
        }
        builder.finish().unwrap();

        // Read and verify all records
        let reader = SstableReader::open(path, config, cache).unwrap();
        for i in 0..50 {
            let key = format!("key_{:03}", i);
            let record = reader.get(&key).unwrap();
            assert!(record.is_some(), "Key {} should exist", key);
        }
    }

    #[test]
    fn test_reader_boundary_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("boundary.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write records with boundary keys
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 111).unwrap();
        builder
            .add(b"aaa", &create_test_record("aaa", b"first"))
            .unwrap();
        builder
            .add(b"mmm", &create_test_record("mmm", b"middle"))
            .unwrap();
        builder
            .add(b"zzz", &create_test_record("zzz", b"last"))
            .unwrap();
        builder.finish().unwrap();

        let reader = SstableReader::open(path, config, cache).unwrap();

        // Test exact boundary keys
        assert!(
            reader.get("aaa").unwrap().is_some(),
            "First key should exist"
        );
        assert!(
            reader.get("zzz").unwrap().is_some(),
            "Last key should exist"
        );

        // Test keys before first
        assert!(
            reader.get("000").unwrap().is_none(),
            "Key before first should not exist"
        );
        assert!(
            reader.get("aa").unwrap().is_none(),
            "Key before first should not exist"
        );

        // Test keys after last
        assert!(
            reader.get("zzzz").unwrap().is_none(),
            "Key after last should not exist"
        );

        // Test keys between boundaries
        assert!(
            reader.get("bbb").unwrap().is_none(),
            "Non-existent key should not exist"
        );
        assert!(
            reader.get("mmm").unwrap().is_some(),
            "Middle key should exist"
        );
    }

    #[test]
    fn test_reader_scan() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("scan_test.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write ordered records
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 999).unwrap();
        let test_keys = vec!["apple", "banana", "cherry"];

        for key in &test_keys {
            builder
                .add(
                    key.as_bytes(),
                    &create_test_record(key, format!("{}_value", key).as_bytes()),
                )
                .unwrap();
        }
        builder.finish().unwrap();

        // Scan all records
        let reader = SstableReader::open(path, config, cache).unwrap();
        let records = reader.scan().unwrap();

        assert_eq!(records.len(), test_keys.len(), "Should scan all records");
    }

    #[test]
    fn test_reader_invalid_magic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("invalid.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write file with wrong magic number
        std::fs::write(&path, b"INVALID_MAGIC").unwrap();

        let result = SstableReader::open(path, config, cache);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LsmError::InvalidSstableFormat(_)
        ));
    }

    #[test]
    fn test_shared_cache_across_readers() {
        let dir = tempdir().unwrap();
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Create two SSTable files
        let path1 = dir.path().join("file1.sst");
        let path2 = dir.path().join("file2.sst");

        // Write first SSTable
        let mut builder1 = SstableBuilder::new(path1.clone(), config.clone(), 111).unwrap();
        builder1
            .add(b"key1", &create_test_record("key1", b"value1"))
            .unwrap();
        builder1.finish().unwrap();

        // Write second SSTable
        let mut builder2 = SstableBuilder::new(path2.clone(), config.clone(), 222).unwrap();
        builder2
            .add(b"key2", &create_test_record("key2", b"value2"))
            .unwrap();
        builder2.finish().unwrap();

        // Open both readers with same cache
        let reader1 = SstableReader::open(path1, config.clone(), Arc::clone(&cache)).unwrap();
        let reader2 = SstableReader::open(path2, config, Arc::clone(&cache)).unwrap();

        let stats_before = cache.stats();

        // Read from first SSTable (populates cache)
        reader1.get("key1").unwrap();
        let stats_after1 = cache.stats();
        assert!(stats_after1.len >= stats_before.len);

        // Read from second SSTable (uses same cache)
        reader2.get("key2").unwrap();
        let stats_after2 = cache.stats();
        assert!(stats_after2.len >= stats_after1.len);

        // Both readers share the same cache
        assert!(stats_after2.len <= stats_after2.cap);
    }

    // ======================
    // CONCURRENCY TESTS
    // ======================

    #[test]
    fn test_concurrent_reads_same_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent_same.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write SSTable with 100 records
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 1000).unwrap();
        for i in 0..100 {
            let key = format!("key_{:03}", i);
            let value = format!("value_{:03}", i);
            builder
                .add(key.as_bytes(), &create_test_record(&key, value.as_bytes()))
                .unwrap();
        }
        builder.finish().unwrap();

        // Open reader and wrap in Arc for sharing
        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());

        // Spawn 10 threads, each reading the same 100 keys 100 times
        let handles: Vec<_> = (0..10)
            .map(|thread_id| {
                let reader_clone = Arc::clone(&reader);
                thread::spawn(move || {
                    for _ in 0..100 {
                        for i in 0..100 {
                            let key = format!("key_{:03}", i);
                            let result = reader_clone.get(&key).unwrap();
                            assert!(
                                result.is_some(),
                                "Thread {} failed to read key {}",
                                thread_id,
                                key
                            );
                        }
                    }
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_reads_different_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent_diff.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write SSTable with 1000 records
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 2000).unwrap();
        for i in 0..1000 {
            let key = format!("key_{:04}", i);
            let value = format!("value_{:04}", i);
            builder
                .add(key.as_bytes(), &create_test_record(&key, value.as_bytes()))
                .unwrap();
        }
        builder.finish().unwrap();

        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());

        // Spawn 10 threads, each reading different ranges of keys
        let handles: Vec<_> = (0..10)
            .map(|thread_id| {
                let reader_clone = Arc::clone(&reader);
                thread::spawn(move || {
                    let start = thread_id * 100;
                    let end = start + 100;
                    for _ in 0..50 {
                        for i in start..end {
                            let key = format!("key_{:04}", i);
                            let result = reader_clone.get(&key).unwrap();
                            assert!(result.is_some(), "Key {} should exist", key);
                            let record = result.unwrap();
                            let expected_value = format!("value_{:04}", i);
                            assert_eq!(record.value, expected_value.as_bytes());
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_reads_with_cache_contention() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent_cache.sst");
        let mut config = StorageConfig::default();
        config.block_size = 512; // Small blocks
        config.block_cache_size_mb = 1; // Small cache to force evictions
        let cache = create_test_cache(&config);

        // Write enough data to span many blocks
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 3000).unwrap();
        for i in 0..500 {
            let key = format!("key_{:04}", i);
            let value = vec![b'x'; 50]; // 50 bytes each
            builder
                .add(key.as_bytes(), &create_test_record(&key, &value))
                .unwrap();
        }
        builder.finish().unwrap();

        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());

        // Spawn threads that cause cache contention
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let reader_clone = Arc::clone(&reader);
                thread::spawn(move || {
                    for _ in 0..200 {
                        // Random-ish access pattern
                        for i in (0..500).step_by(7) {
                            let key = format!("key_{:04}", i);
                            let result = reader_clone.get(&key);
                            assert!(result.is_ok());
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_readers_shared_cache() {
        let dir = tempdir().unwrap();
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Create 3 SSTable files
        let paths: Vec<_> = (0..3)
            .map(|i| {
                let path = dir.path().join(format!("file_{}.sst", i));
                let mut builder =
                    SstableBuilder::new(path.clone(), config.clone(), 4000 + i).unwrap();
                for j in 0..100 {
                    let key = format!("key_{}_{:03}", i, j);
                    let value = format!("value_{}_{:03}", i, j);
                    builder
                        .add(key.as_bytes(), &create_test_record(&key, value.as_bytes()))
                        .unwrap();
                }
                builder.finish().unwrap();
                path
            })
            .collect();

        // Open 3 readers with shared cache
        let readers: Vec<_> = paths
            .into_iter()
            .map(|path| {
                Arc::new(
                    SstableReader::open(path, config.clone(), Arc::clone(&cache)).unwrap(),
                )
            })
            .collect();

        // Spawn threads that read from different SSTables concurrently
        let handles: Vec<_> = (0..9)
            .map(|thread_id| {
                let reader_idx = thread_id % 3;
                let reader_clone = Arc::clone(&readers[reader_idx]);
                thread::spawn(move || {
                    for _ in 0..100 {
                        for j in 0..100 {
                            let key = format!("key_{}_{:03}", reader_idx, j);
                            let result = reader_clone.get(&key).unwrap();
                            assert!(result.is_some(), "Key {} should exist", key);
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_concurrent_scan() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("concurrent_scan.sst");
        let config = StorageConfig::default();
        let cache = create_test_cache(&config);

        // Write SSTable
        let mut builder = SstableBuilder::new(path.clone(), config.clone(), 5000).unwrap();
        for i in 0..200 {
            let key = format!("key_{:03}", i);
            let value = format!("value_{:03}", i);
            builder
                .add(key.as_bytes(), &create_test_record(&key, value.as_bytes()))
                .unwrap();
        }
        builder.finish().unwrap();

        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());

        // Spawn 5 threads all doing full scans simultaneously
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let reader_clone = Arc::clone(&reader);
                thread::spawn(move || {
                    for _ in 0..10 {
                        let records = reader_clone.scan().unwrap();
                        assert_eq!(records.len(), 200, "Should scan all 200 records");
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }
    }
}
