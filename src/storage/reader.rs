use crate::core::log_record::LogRecord;
use crate::infra::codec::decode;
use crate::infra::config::StorageConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::block::Block;
use crate::storage::builder::{BlockMeta, MetaBlock};
use crate::storage::cache::GlobalBlockCache;
use bloomfilter::Bloom;
use lz4_flex::decompress_size_prepended;
use parking_lot::Mutex;
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
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
    table_id: u64,
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

        // Generate table ID from path for cache
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let table_id = hasher.finish();

        Ok(Self {
            metadata,
            bloom_filter,
            file: Mutex::new(file),
            block_cache,
            path,
            table_id,
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

        // Read block by offset (no lock needed - immutable metadata)
        let block_meta = match self.read_block_by_key(key.as_bytes()) {
            Some(meta) => meta,
            None => return Ok(None),
        };

        // Read and decompress the block (with caching)
        let block_data = self.read_block(&block_meta)?;

        // Deserialize block (no lock needed)
        let block = Block::decode(&block_data)?;

        // Linear scan within the block to find the key (no lock needed)
        Self::search_in_block(&block, key.as_bytes())
    }

    /// Search for a key within a decoded block
    pub(crate) fn search_in_block(block: &Block, key: &[u8]) -> Result<Option<LogRecord>> {
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
        self.scan_range(None, None)
    }

    /// Scan records in the SSTable within the given range.
    ///
    /// This method efficiently skips blocks that are entirely before the start_key
    /// using the sparse index stored in metadata. This provides O(num_blocks_in_range)
    /// complexity instead of O(total_blocks) for full scans.
    ///
    /// # Arguments
    /// * `start` - Inclusive lower bound (None = from first key)
    /// * `end` - Exclusive upper bound (None = to last key)
    ///
    /// # Performance Note
    ///
    /// This method uses the sparse index to skip blocks before start_key. However,
    /// once we reach the appropriate blocks, we still read all entries in each block
    /// and filter by key. True O(result_count) complexity would require:
    /// 1. A denser index with every k-th key (currently we have first_key per block)
    /// 2. Binary search within blocks to find exact entry positions
    ///
    /// Current complexity: O(blocks_before_start + blocks_in_range * entries_per_block)
    /// For typical block sizes (~256-512 bytes), this is much better than full scan.
    ///
    /// # Thread Safety
    /// This method can be safely called concurrently from multiple threads.
    pub fn scan_range(
        &self,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<Vec<(Vec<u8>, LogRecord)>> {
        let mut records = Vec::new();

        // Find starting block using sparse index
        let start_block_idx = if let Some(start_key) = start {
            // Binary search for the first block where first_key > start_key
            // and then step back by 1 to get the block that could contain start_key.
            let idx = self.metadata.blocks.partition_point(|block| {
                // Use bytes comparison to avoid String allocation
                block.first_key.as_slice() <= start_key.as_bytes()
            });
            if idx == 0 { 0 } else { idx - 1 }
        } else {
            // Start from first block
            0
        };

        // Iterate through blocks starting from the right position
        for block_meta in &self.metadata.blocks[start_block_idx..] {
            // Check if we've passed the end key
            if let Some(end_key) = end {
                // If the block's first key is >= end, we're done with this SSTable
                if block_meta.first_key.as_slice() >= end_key.as_bytes() {
                    break;
                }
            }

            let block_data = self.read_block(block_meta)?;
            let block = Block::decode(&block_data)?;

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

                // Check start filter (exclusive start for pagination)
                if let Some(start_key) = start {
                    if key.as_slice() <= start_key.as_bytes() {
                        continue;
                    }
                }

                // Check end filter
                if let Some(end_key) = end {
                    if key.as_slice() >= end_key.as_bytes() {
                        // Keys in a block are sorted, so we can break early
                        break;
                    }
                }

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

    /// Read and decompress a block by its metadata.
    /// Results are cached in the shared `GlobalBlockCache`.
    ///
    /// Exposed as `pub(crate)` so that `SstableIterator` can load blocks
    /// without duplicating the decompression + cache logic.
    pub(crate) fn read_block(&self, block_meta: &BlockMeta) -> Result<Vec<u8>> {
        // Use block index as cache key (blocks are numbered 0, 1, 2...)
        let block_idx = self
            .metadata
            .blocks
            .iter()
            .position(|b| b.offset == block_meta.offset)
            .unwrap_or(0);

        // Check shared cache first (GlobalBlockCache has internal Mutex)
        if let Some(cached) = self.block_cache.get(self.table_id, block_idx) {
            return Ok(cached);
        }

        // Cache miss - read from disk (lock released during decompression)
        let block_data = self.read_and_decompress_block(block_meta)?;

        // Store in shared cache (GlobalBlockCache has internal Mutex)
        self.block_cache
            .put(self.table_id, block_idx, block_data.clone());

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

    /// Find block metadata by key using sparse index.
    /// Returns the block metadata if the key might exist in this block.
    fn read_block_by_key(&self, key: &[u8]) -> Option<BlockMeta> {
        // Binary search for the first block where first_key > key
        let idx = self
            .metadata
            .blocks
            .partition_point(|block| block.first_key.as_slice() <= key);

        if idx == 0 {
            // Key is before first block's first_key; no block can contain it
            return None;
        }

        // Candidate block is idx - 1
        self.metadata.blocks.get(idx - 1).cloned()
    }
}
