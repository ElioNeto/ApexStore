//! SSTable Iterator
//!
//! Provides [`SstableIterator`] which implements [`StorageIterator`] for
//! disk-based, sorted iteration over SSTable files.
//!
//! Blocks are loaded **lazily** from disk (or the shared [`GlobalBlockCache`])
//! as the iterator advances across block boundaries, keeping memory usage low.

use crate::core::log_record::LogRecord;
use crate::infra::codec::decode;
use crate::infra::error::Result;
use crate::storage::block::Block;
use crate::storage::iterator::StorageIterator;
use crate::storage::reader::SstableReader;
use std::sync::Arc;

/// Iterator over all key-value pairs stored in an SSTable file.
///
/// Iterates in **sorted key order**. Blocks are loaded one at a time from
/// disk (or the shared block cache) as the iterator crosses block boundaries.
///
/// # Usage
///
/// ```ignore
/// let mut iter = SstableIterator::new(Arc::clone(&reader))?;
/// iter.seek(b"start_key");
/// while iter.is_valid() {
///     println!("{}", String::from_utf8_lossy(iter.key()));
///     iter.next();
/// }
/// ```
///
/// # Thread Safety
///
/// `SstableIterator` itself is **not** `Send`/`Sync` — each thread should
/// construct its own iterator. The underlying `Arc<SstableReader>` is
/// thread-safe and can be shared freely.
pub struct SstableIterator {
    reader: Arc<SstableReader>,
    /// Index into `reader.metadata().blocks` for the currently loaded block.
    block_index: usize,
    /// Index into `current_block.offsets` for the current entry.
    offset_index: usize,
    /// Currently decoded block held in memory.
    current_block: Option<Block>,
    /// Decoded key bytes of the current entry.
    current_key: Vec<u8>,
    /// Decoded value (LogRecord) of the current entry.
    /// `None` means the iterator is exhausted / invalid.
    current_value: Option<LogRecord>,
}

impl SstableIterator {
    // ── Constructors ──────────────────────────────────────────────────────

    /// Create an iterator positioned at the **first** entry of the SSTable.
    ///
    /// Returns `Err` only if reading the first block from disk fails.
    pub fn new(reader: Arc<SstableReader>) -> Result<Self> {
        let mut iter = Self {
            reader,
            block_index: 0,
            offset_index: 0,
            current_block: None,
            current_key: Vec::new(),
            current_value: None,
        };
        iter.load_block(0)?;
        iter.load_current_entry();
        Ok(iter)
    }

    /// Create an iterator positioned at the first key **>= `key`**.
    ///
    /// Equivalent to `SstableIterator::new(reader)` followed by `seek(key)`.
    pub fn new_seek(reader: Arc<SstableReader>, key: &[u8]) -> Result<Self> {
        let mut iter = Self::new(reader)?;
        iter.seek(key);
        Ok(iter)
    }

    // ── Private helpers ───────────────────────────────────────────────────

    /// Load block `block_idx` from the reader into `self.current_block`.
    /// Resets `offset_index` to 0. Invalidates the iterator if out of bounds.
    fn load_block(&mut self, block_idx: usize) -> Result<()> {
        let meta = self.reader.metadata();
        if block_idx >= meta.blocks.len() {
            self.current_block = None;
            self.invalidate();
            return Ok(());
        }
        let block_meta = meta.blocks[block_idx].clone();
        let raw = self.reader.read_block(&block_meta)?;
        self.current_block = Some(Block::decode(&raw));
        self.block_index = block_idx;
        self.offset_index = 0;
        Ok(())
    }

    /// Decode the entry at `current_block.offsets[offset_index]` and store
    /// it into `current_key` / `current_value`.
    fn load_current_entry(&mut self) {
        let result = self.current_block.as_ref().and_then(|block| {
            if self.offset_index >= block.offsets.len() {
                return None;
            }
            let off = block.offsets[self.offset_index] as usize;
            Self::read_entry(&block.data, off)
        });

        match result {
            None => self.invalidate(),
            Some((key, val_bytes)) => match decode::<LogRecord>(&val_bytes) {
                Ok(record) => {
                    self.current_key = key;
                    self.current_value = Some(record);
                }
                Err(_) => self.invalidate(),
            },
        }
    }

    /// Clear current key/value, marking the iterator as exhausted.
    fn invalidate(&mut self) {
        self.current_key.clear();
        self.current_value = None;
    }

    /// Determine the index of the block that *could* contain `key`.
    /// Uses `partition_point` on the sparse index (O(log B)).
    fn find_block_for_key(&self, key: &[u8]) -> usize {
        let blocks = &self.reader.metadata().blocks;
        if blocks.is_empty() {
            return 0;
        }
        let idx = blocks.partition_point(|bm| bm.first_key.as_slice() <= key);
        if idx == 0 {
            0
        } else {
            idx - 1
        }
    }

    // ── Low-level byte parsers ────────────────────────────────────────────

    /// Parse `(key_bytes, value_bytes)` from `data` at `offset`.
    ///
    /// Block entry layout:
    /// ```text
    /// [ key_len: u16 ][ key: [u8; key_len] ][ val_len: u16 ][ val: [u8; val_len] ]
    /// ```
    ///
    /// Returns `None` if `data` is too short (bounds check).
    fn read_entry(data: &[u8], offset: usize) -> Option<(Vec<u8>, Vec<u8>)> {
        if offset + 4 > data.len() {
            return None;
        }
        let key_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        let key_end = offset + 2 + key_len;
        if key_end + 2 > data.len() {
            return None;
        }
        let key = data[offset + 2..key_end].to_vec();
        let val_len = u16::from_le_bytes([data[key_end], data[key_end + 1]]) as usize;
        let val_end = key_end + 2 + val_len;
        if val_end > data.len() {
            return None;
        }
        Some((key, data[key_end + 2..val_end].to_vec()))
    }

    /// Parse only the key from `data` at `offset` (skips the value).
    /// Returns `None` if `data` is too short.
    fn read_key(data: &[u8], offset: usize) -> Option<Vec<u8>> {
        if offset + 2 > data.len() {
            return None;
        }
        let key_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        let key_end = offset + 2 + key_len;
        if key_end > data.len() {
            return None;
        }
        Some(data[offset + 2..key_end].to_vec())
    }
}

impl StorageIterator for SstableIterator {
    /// Current key bytes. **Panics** if the iterator is invalid.
    fn key(&self) -> &[u8] {
        &self.current_key
    }

    /// Current value (LogRecord). **Panics** if the iterator is invalid.
    fn value(&self) -> &LogRecord {
        self.current_value
            .as_ref()
            .expect("value() called on invalid SstableIterator")
    }

    /// Returns `true` while the iterator points to a valid entry.
    fn is_valid(&self) -> bool {
        self.current_value.is_some()
    }

    /// Advance to the next entry.
    ///
    /// Transparently loads the next block when the current one is exhausted.
    /// Calling `next()` on an already-invalid iterator is a no-op.
    fn next(&mut self) {
        if !self.is_valid() {
            return;
        }

        let at_last = match &self.current_block {
            Some(b) => self.offset_index + 1 >= b.offsets.len(),
            None => true,
        };

        if at_last {
            let next_idx = self.block_index + 1;
            if self.load_block(next_idx).is_err() {
                self.invalidate();
                return;
            }
            // load_block already reset offset_index to 0
        } else {
            self.offset_index += 1;
        }

        self.load_current_entry();
    }

    /// Seek to the first entry with key **>= `key`**.
    ///
    /// Uses the SSTable sparse index (binary search, O(log B)) to jump to the
    /// candidate block, then performs a linear scan within that block.
    /// If no such key exists the iterator becomes invalid.
    fn seek(&mut self, key: &[u8]) {
        let block_idx = self.find_block_for_key(key);

        if self.load_block(block_idx).is_err() {
            self.invalidate();
            return;
        }

        // Linear scan forward until we find key >= target.
        loop {
            let (entry_key, exhausted) = match &self.current_block {
                None => {
                    self.invalidate();
                    return;
                }
                Some(block) => {
                    if self.offset_index >= block.offsets.len() {
                        (None, true)
                    } else {
                        let off = block.offsets[self.offset_index] as usize;
                        (Self::read_key(&block.data, off), false)
                    }
                }
            };

            if exhausted {
                let next_idx = self.block_index + 1;
                if next_idx >= self.reader.metadata().blocks.len() {
                    self.invalidate();
                    return;
                }
                if self.load_block(next_idx).is_err() {
                    self.invalidate();
                    return;
                }
                continue;
            }

            match entry_key {
                Some(ek) if ek.as_slice() >= key => break,
                Some(_) => self.offset_index += 1,
                None => {
                    self.invalidate();
                    return;
                }
            }
        }

        self.load_current_entry();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::StorageConfig;
    use crate::storage::builder::SstableBuilder;
    use crate::storage::cache::GlobalBlockCache;
    use tempfile::tempdir;

    // ── Test helpers ──────────────────────────────────────────────────────

    fn make_cache(config: &StorageConfig) -> Arc<GlobalBlockCache> {
        GlobalBlockCache::new(config.block_cache_size_mb, config.block_size)
    }

    fn make_record(key: &str, value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_string(), value.to_vec())
    }

    /// Write `n` records (key_000 … key_{n-1}) and return an Arc<SstableReader>.
    /// `_dir` must be kept alive for the file to remain accessible.
    fn write_n(
        n: usize,
        block_size: usize,
    ) -> (Arc<SstableReader>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("t.sst");
        let config = StorageConfig {
            block_size,
            ..Default::default()
        };
        let cache = make_cache(&config);
        let mut b = SstableBuilder::new(path.clone(), config.clone(), 1).unwrap();
        for i in 0..n {
            let k = format!("key_{:03}", i);
            let v = format!("val_{:03}", i);
            b.add(k.as_bytes(), &make_record(&k, v.as_bytes())).unwrap();
        }
        b.finish().unwrap();
        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());
        (reader, dir)
    }

    // ── Construction ──────────────────────────────────────────────────────

    #[test]
    fn test_new_positions_at_first_entry() {
        let (r, _d) = write_n(5, 4096);
        let iter = SstableIterator::new(r).unwrap();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_000");
    }

    #[test]
    fn test_new_seek_constructor() {
        let (r, _d) = write_n(20, 4096);
        let iter = SstableIterator::new_seek(Arc::clone(&r), b"key_010").unwrap();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_010");
    }

    // ── Full traversal ────────────────────────────────────────────────────

    #[test]
    fn test_full_scan_single_block_key_order() {
        let n = 10;
        let (r, _d) = write_n(n, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        let mut count = 0usize;
        while iter.is_valid() {
            assert_eq!(iter.key(), format!("key_{:03}", count).as_bytes());
            count += 1;
            iter.next();
        }
        assert_eq!(count, n);
    }

    #[test]
    fn test_full_scan_multi_block_key_order() {
        let n = 50;
        let (r, _d) = write_n(n, 256);
        let mut iter = SstableIterator::new(r).unwrap();
        let mut count = 0usize;
        while iter.is_valid() {
            assert_eq!(
                iter.key(),
                format!("key_{:03}", count).as_bytes(),
                "Mismatch at idx {}",
                count
            );
            count += 1;
            iter.next();
        }
        assert_eq!(count, n);
    }

    #[test]
    fn test_full_scan_values_correct() {
        let n = 20;
        let (r, _d) = write_n(n, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        let mut idx = 0usize;
        while iter.is_valid() {
            assert_eq!(iter.value().value, format!("val_{:03}", idx).as_bytes());
            idx += 1;
            iter.next();
        }
        assert_eq!(idx, n);
    }

    // ── seek() ────────────────────────────────────────────────────────────

    #[test]
    fn test_seek_exact_first_key() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_000");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_000");
    }

    #[test]
    fn test_seek_exact_last_key_then_exhausted() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_009");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_009");
        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_seek_exact_middle_key() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_005");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_005");
    }

    #[test]
    fn test_seek_between_keys_lands_on_successor() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        // key_0055 does not exist → should land on key_006
        iter.seek(b"key_0055");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_006");
    }

    #[test]
    fn test_seek_before_first_lands_on_first() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"aaa"); // lexicographically before all keys
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_000");
    }

    #[test]
    fn test_seek_after_last_invalidates_iterator() {
        let (r, _d) = write_n(10, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"zzz"); // after all keys
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_seek_multi_block_cross_boundary() {
        let n = 50;
        let (r, _d) = write_n(n, 256); // forces multiple blocks
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_030");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_030");
        // Continue iterating forward
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_031");
    }

    // ── Scan from seek point ──────────────────────────────────────────────

    #[test]
    fn test_scan_from_seek_yields_tail() {
        let n = 10;
        let (r, _d) = write_n(n, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_005");
        let mut count = 5usize;
        while iter.is_valid() {
            assert_eq!(iter.key(), format!("key_{:03}", count).as_bytes());
            count += 1;
            iter.next();
        }
        assert_eq!(count, n);
    }

    // ── next() past end ───────────────────────────────────────────────────

    #[test]
    fn test_next_past_end_is_noop() {
        let (r, _d) = write_n(2, 4096);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.next();
        iter.next(); // now invalid
        assert!(!iter.is_valid());
        iter.next(); // should not panic
        assert!(!iter.is_valid());
    }

    // ── Large SSTable ─────────────────────────────────────────────────────

    #[test]
    fn test_large_sst_full_iteration_count() {
        let n = 200;
        let (r, _d) = write_n(n, 512);
        let mut iter = SstableIterator::new(r).unwrap();
        let mut count = 0usize;
        while iter.is_valid() {
            count += 1;
            iter.next();
        }
        assert_eq!(count, n);
    }

    #[test]
    fn test_large_sst_seek_last_quarter() {
        let n = 200;
        let (r, _d) = write_n(n, 512);
        let mut iter = SstableIterator::new(r).unwrap();
        iter.seek(b"key_150");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_150");
        // Count remaining entries: key_150 … key_199 = 50
        let mut count = 0usize;
        while iter.is_valid() {
            count += 1;
            iter.next();
        }
        assert_eq!(count, 50);
    }

    // ── Tombstones ────────────────────────────────────────────────────────

    #[test]
    fn test_tombstones_visible_through_iterator() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tomb.sst");
        let config = StorageConfig::default();
        let cache = make_cache(&config);

        // Keys in sorted order: alive < also_alive < dead
        let mut b = SstableBuilder::new(path.clone(), config.clone(), 42).unwrap();
        b.add(b"alive", &make_record("alive", b"ok")).unwrap();
        b.add(b"also_alive", &make_record("also_alive", b"ok2"))
            .unwrap();
        b.add(b"dead", &LogRecord::tombstone("dead".to_string()))
            .unwrap();
        b.finish().unwrap();

        let reader = Arc::new(SstableReader::open(path, config, cache).unwrap());
        let mut iter = SstableIterator::new(reader).unwrap();

        assert_eq!(iter.key(), b"alive");
        assert!(!iter.value().is_deleted);

        iter.next();
        assert_eq!(iter.key(), b"also_alive");
        assert!(!iter.value().is_deleted);

        iter.next();
        assert_eq!(iter.key(), b"dead");
        assert!(iter.value().is_deleted);

        iter.next();
        assert!(!iter.is_valid());
    }

    // ── Low-level helpers ─────────────────────────────────────────────────

    #[test]
    fn test_read_entry_happy_path() {
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&3u16.to_le_bytes()); // key_len = 3
        data.extend_from_slice(b"foo");
        data.extend_from_slice(&3u16.to_le_bytes()); // val_len = 3
        data.extend_from_slice(b"bar");

        let (key, val) = SstableIterator::read_entry(&data, 0).unwrap();
        assert_eq!(key, b"foo");
        assert_eq!(val, b"bar");
    }

    #[test]
    fn test_read_entry_truncated_returns_none() {
        // Claims key_len=10 but data is only 2 bytes
        let data = 10u16.to_le_bytes().to_vec();
        assert!(SstableIterator::read_entry(&data, 0).is_none());
    }

    #[test]
    fn test_read_entry_empty_data_returns_none() {
        assert!(SstableIterator::read_entry(&[], 0).is_none());
    }

    #[test]
    fn test_read_key_happy_path() {
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&5u16.to_le_bytes());
        data.extend_from_slice(b"hello");
        data.extend_from_slice(&0u16.to_le_bytes()); // val_len irrelevant

        assert_eq!(
            SstableIterator::read_key(&data, 0),
            Some(b"hello".to_vec())
        );
    }

    #[test]
    fn test_read_key_truncated_returns_none() {
        // Claims key_len=5 but only 1 byte of key present
        let mut data: Vec<u8> = Vec::new();
        data.extend_from_slice(&5u16.to_le_bytes());
        data.push(b'a'); // only 1 byte instead of 5
        assert!(SstableIterator::read_key(&data, 0).is_none());
    }

    #[test]
    fn test_read_key_empty_data_returns_none() {
        assert!(SstableIterator::read_key(&[], 0).is_none());
    }

    // ── find_block_for_key ────────────────────────────────────────────────

    #[test]
    fn test_find_block_always_returns_valid_index() {
        let (r, _d) = write_n(20, 256); // multiple blocks
        let iter = SstableIterator::new(r).unwrap();
        // Should not panic or return out-of-bounds index
        let idx = iter.find_block_for_key(b"key_010");
        assert!(idx < iter.reader.metadata().blocks.len());
    }
}
