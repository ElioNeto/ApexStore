use crate::core::log_record::LogRecord;
use crate::core::memtable::MemTable;
use crate::infra::config::LsmConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::builder::SstableBuilder;
use crate::storage::cache::GlobalBlockCache;
use crate::storage::reader::SstableReader;
use crate::storage::wal::WriteAheadLog;

use parking_lot::{Mutex, RwLock};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{info, warn};

/// Maximum number of records to return in a single scan/prefix search
const MAX_SCAN_LIMIT: usize = 10000;
const DEFAULT_SCAN_LIMIT: usize = 1000;

#[derive(Serialize)]
pub struct LsmStats {
    pub mem_records: usize,
    pub mem_kb: usize,
    pub sst_files: usize,
    pub sst_records: u64,
    pub sst_kb: u64,
    pub wal_kb: u64,
    pub total_records: u64,
    pub memtable_max_size: usize,
}

/// Core LSM-tree storage engine.
///
/// # Concurrency Model
///
/// - `memtable` uses a `parking_lot::Mutex`.  Writes and reads both take
///   an exclusive lock; contention is low because MemTable operations are
///   in-memory and sub-microsecond.
///
/// - `sstables` uses a `parking_lot::RwLock`.  Read operations (`get`,
///   `scan`, `stats`) take a **shared** read lock, allowing full read
///   concurrency.  Only `flush()` takes an exclusive write lock to insert
///   a new SSTable at the front of the list.
///
/// Both lock types are non-poisoning (`parking_lot` guarantee), so there
/// is no need to handle `PoisonError` anywhere in this module.
pub struct LsmEngine {
    memtable: Mutex<MemTable>,
    wal: WriteAheadLog,
    sstables: RwLock<Vec<SstableReader>>,
    block_cache: Arc<GlobalBlockCache>,
    pub(crate) dir_path: PathBuf,
    pub(crate) config: LsmConfig,
}

impl LsmEngine {
    pub fn new(config: LsmConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.core.dir_path)?;

        let block_cache = GlobalBlockCache::new(
            config.storage.block_cache_size_mb,
            config.storage.block_size,
        );

        let wal = WriteAheadLog::new(&config.core.dir_path)?;
        let wal_records = wal.recover()?;

        let mut sstables = Vec::new();
        for entry in std::fs::read_dir(&config.core.dir_path)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sst") {
                match SstableReader::open(
                    path.clone(),
                    config.storage.clone(),
                    Arc::clone(&block_cache),
                ) {
                    Ok(sst) => sstables.push(sst),
                    Err(e) => warn!("Failed to load SSTable {}: {}", path.display(), e),
                }
            }
        }

        // Newest-first: ensures get() returns the most recent version
        sstables.sort_by(|a, b| b.metadata().timestamp.cmp(&a.metadata().timestamp));

        let mut memtable = MemTable::new(config.core.memtable_max_size);
        for record in wal_records {
            memtable.insert(record);
        }

        info!(
            "LSM Engine initialized: {} sstables, memtable={} records, cache={}MB",
            sstables.len(),
            memtable.data.len(),
            config.storage.block_cache_size_mb
        );

        Ok(Self {
            memtable: Mutex::new(memtable),
            wal,
            sstables: RwLock::new(sstables),
            block_cache,
            dir_path: config.core.dir_path.clone(),
            config,
        })
    }

    // -------------------------------------------------------------------------
    // Write path
    // -------------------------------------------------------------------------

    pub fn set(&self, key: String, value: Vec<u8>) -> Result<()> {
        let record = LogRecord::new(key, value);
        self.wal.write_record(&record)?;

        let mut memtable = self.memtable.lock();
        memtable.insert(record);

        if memtable.should_flush() {
            drop(memtable);
            self.flush()?;
        }

        Ok(())
    }

    pub fn delete(&self, key: String) -> Result<()> {
        let record = LogRecord::tombstone(key);
        self.wal.write_record(&record)?;

        let mut memtable = self.memtable.lock();
        memtable.insert(record);

        if memtable.should_flush() {
            drop(memtable);
            self.flush()?;
        }

        Ok(())
    }

    /// Insert a batch of key-value pairs atomically.
    ///
    /// # Atomicity Guarantee
    ///
    /// All WAL writes complete before the MemTable is touched.  If any WAL
    /// write fails, the MemTable remains unmodified and the caller can safely
    /// retry the entire batch.  Under normal operation this eliminates the
    /// partial-write inconsistency present in the previous N-sequential-set
    /// implementation.
    ///
    /// The batch is written with a **single fsync** at the end of the WAL
    /// phase and a **single MemTable lock acquisition**, reducing contention
    /// and I/O overhead from O(N) to O(1).
    ///
    /// Note: full crash-atomic all-or-nothing semantics (batch-begin /
    /// batch-commit WAL markers) is a future enhancement.
    pub fn set_batch(&self, items: Vec<(String, Vec<u8>)>) -> Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }

        // 1. Build records first so validation errors abort before any I/O.
        let records: Vec<LogRecord> = items
            .into_iter()
            .map(|(key, value)| LogRecord::new(key, value))
            .collect();

        // 2. Write every record to the WAL.  A failure here means zero
        //    MemTable mutations have occurred; the caller may retry.
        for record in &records {
            self.wal.write_record(record)?;
        }

        // 3. Acquire the MemTable lock once and insert all records.
        let count = records.len();
        let should_flush = {
            let mut memtable = self.memtable.lock();
            for record in records {
                memtable.insert(record);
            }
            memtable.should_flush()
        }; // lock released here

        if should_flush {
            self.flush()?;
        }

        Ok(count)
    }

    /// Delete a batch of keys atomically.
    ///
    /// Follows the same atomicity contract as `set_batch`: all tombstone
    /// records are written to the WAL before the MemTable is touched.
    pub fn delete_batch(&self, keys: Vec<String>) -> Result<usize> {
        if keys.is_empty() {
            return Ok(0);
        }

        let records: Vec<LogRecord> = keys.into_iter().map(LogRecord::tombstone).collect();

        for record in &records {
            self.wal.write_record(record)?;
        }

        let count = records.len();
        let should_flush = {
            let mut memtable = self.memtable.lock();
            for record in records {
                memtable.insert(record);
            }
            memtable.should_flush()
        };

        if should_flush {
            self.flush()?;
        }

        Ok(count)
    }

    // -------------------------------------------------------------------------
    // Read path
    // -------------------------------------------------------------------------

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // Check MemTable first (most recent data).
        {
            let memtable = self.memtable.lock();
            if let Some(record) = memtable.get(key) {
                return Ok(if record.is_deleted {
                    None
                } else {
                    Some(record.value)
                });
            }
        }

        // Check SSTables newest-to-oldest under a shared read lock.
        let sstables = self.sstables.read();
        for sst in sstables.iter() {
            if let Some(record) = sst.get(key)? {
                return Ok(if record.is_deleted {
                    None
                } else {
                    Some(record.value)
                });
            }
        }

        Ok(None)
    }

    pub fn search(&self, pattern: &str) -> Result<Vec<(String, Vec<u8>)>> {
        Ok(self
            .scan()?
            .into_iter()
            .filter(|(key, _)| key.contains(pattern))
            .collect())
    }

    /// Legacy prefix search (full scan, no pagination) - kept for backwards compatibility
    #[deprecated(since = "2.2.0", note = "Use search_prefix with pagination instead")]
    pub fn search_prefix_legacy(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        Ok(self
            .scan()?
            .into_iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .collect())
    }

    pub fn scan(&self) -> Result<Vec<(String, Vec<u8>)>> {
        // HashMap keyed by String; value is (bytes, timestamp, is_deleted).
        // MemTable entries win over SSTable entries for the same key because
        // they are inserted first and `entry().or_insert()` never overwrites.
        let mut result_map: HashMap<String, (Vec<u8>, u128, bool)> = HashMap::new();

        {
            let memtable = self.memtable.lock();
            for (key, record) in memtable.iter_ordered() {
                result_map.insert(
                    key.clone(),
                    (record.value.clone(), record.timestamp, record.is_deleted),
                );
            }
        }

        {
            let sstables = self.sstables.read();
            for sst in sstables.iter() {
                for (key_bytes, record) in sst.scan()? {
                    let key = String::from_utf8(key_bytes)
                        .map_err(|e| LsmError::CorruptedData(e.to_string()))?;
                    result_map.entry(key).or_insert((
                        record.value,
                        record.timestamp,
                        record.is_deleted,
                    ));
                }
            }
        }

        let mut results: Vec<(String, Vec<u8>)> = result_map
            .into_iter()
            .filter_map(|(key, (value, _ts, is_deleted))| {
                if !is_deleted {
                    Some((key, value))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }

    pub fn keys(&self) -> Result<Vec<String>> {
        Ok(self.scan()?.into_iter().map(|(k, _)| k).collect())
    }

    pub fn count(&self) -> Result<usize> {
        Ok(self.scan()?.len())
    }

    // -------------------------------------------------------------------------
    // Range Scan & Prefix Search
    // -------------------------------------------------------------------------

    /// Returns up to `limit` key-value pairs in range [start, end).
    /// If `start` is None, start from first key.
    /// If `end` is None, continue until limit is reached.
    /// Returns (items, next_cursor) where next_cursor is the last returned key (if any).
    pub fn scan_range(
        &self,
        start: Option<&str>,
        end: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<(String, Vec<u8>)>, Option<String>)> {
        if limit == 0 {
            return Err(LsmError::InvalidArgument(
                "limit must be greater than 0".to_string(),
            ));
        }
        if limit > MAX_SCAN_LIMIT {
            return Err(LsmError::InvalidArgument(format!(
                "limit {} exceeds maximum allowed limit {}",
                limit, MAX_SCAN_LIMIT
            )));
        }
        // Validate end > start if both are provided
        if let (Some(start_key), Some(end_key)) = (start, end) {
            if start_key >= end_key {
                return Err(LsmError::InvalidArgument(format!(
                    "start_key '{}' must be less than end_key '{}'",
                    start_key, end_key
                )));
            }
        }

        let mut seen_keys: BTreeMap<String, Vec<u8>> = BTreeMap::new();

        // Collect from MemTable (in sorted order)
        {
            let memtable = self.memtable.lock();

            for (key, record) in memtable.iter_ordered() {
                // Skip if key is before start range
                if let Some(s) = start {
                    if key.as_str() < s {
                        continue;
                    }
                }

                // Stop if we've reached end range
                if let Some(e) = end {
                    if key.as_str() >= e {
                        break;
                    }
                }

                // Skip tombstones
                if record.is_deleted {
                    continue;
                }

                // MemTable wins over SSTable for same key
                seen_keys.insert(key.clone(), record.value.clone());
            }
        }

        // Collect from SSTables (oldest first to ensure MemTable wins by insertion order)
        let sstables = self.sstables.read();
        for sst in sstables.iter() {
            // Scan range in SSTable and merge with results
            let sst_scan = sst.scan()?;
            for (key_bytes, record) in sst_scan {
                let key = String::from_utf8(key_bytes)
                    .map_err(|e| LsmError::CorruptedData(e.to_string()))?;

                // Skip if in range check
                if let Some(s) = start {
                    if key.as_str() < s {
                        continue;
                    }
                }
                if let Some(e) = end {
                    if key.as_str() >= e {
                        break;
                    }
                }

                // Skip if already seen (MemTable wins) or if we have enough
                if seen_keys.contains_key(&key) {
                    continue;
                }

                if record.is_deleted {
                    continue;
                }

                seen_keys.insert(key, record.value);
            }
            // Check if we've reached limit across all SSTables
            if seen_keys.len() >= limit {
                break;
            }
        }

        // Convert to Vec with limit applied
        let results: Vec<(String, Vec<u8>)> = seen_keys
            .into_iter()
            .take(limit)
            .collect();

        // Determine next cursor for pagination
        let next_cursor = if results.len() == limit && !results.is_empty() {
            // Could be more results; return last key as cursor
            results.last().map(|(k, _)| k.clone())
        } else {
            None
        };

        Ok((results, next_cursor))
    }

    /// Returns up to `limit` keys with the given prefix, starting after `cursor`.
    pub fn search_prefix(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<(String, Vec<u8>)>, Option<String>)> {
        if limit == 0 {
            return Err(LsmError::InvalidArgument(
                "limit must be greater than 0".to_string(),
            ));
        }
        if limit > MAX_SCAN_LIMIT {
            return Err(LsmError::InvalidArgument(format!(
                "limit {} exceeds maximum allowed limit {}",
                limit, MAX_SCAN_LIMIT
            )));
        }

        let mut seen_keys: BTreeMap<String, Vec<u8>> = BTreeMap::new();

        // Collect from MemTable (sorted order)
        {
            let memtable = self.memtable.lock();

            for (key, record) in memtable.iter_ordered() {
                // Skip keys before cursor (cursor is exclusive)
                if let Some(cur) = cursor {
                    if key.as_str() <= cur {
                        continue;
                    }
                }

                // Check: key must have the prefix
                if !key.starts_with(prefix) {
                    continue;
                }

                // Skip tombstones
                if record.is_deleted {
                    continue;
                }

                seen_keys.insert(key.clone(), record.value.clone());
            }
        }

        // Collect from SSTables
        let sstables = self.sstables.read();
        for sst in sstables.iter() {
            let sst_scan = sst.scan()?;
            for (key_bytes, record) in sst_scan {
                let key = String::from_utf8(key_bytes)
                    .map_err(|e| LsmError::CorruptedData(e.to_string()))?;

                // Skip keys before cursor (cursor is exclusive)
                if let Some(cur) = cursor {
                    if key.as_str() <= cur {
                        continue;
                    }
                }

                // Check: key must have the prefix
                if !key.starts_with(prefix) {
                    continue;
                }

                // Skip if already in seen_keys (MemTable wins)
                if seen_keys.contains_key(&key) {
                    continue;
                }

                if record.is_deleted {
                    continue;
                }

                seen_keys.insert(key, record.value);
            }
        }

        // Convert to Vec with limit
        let results: Vec<(String, Vec<u8>)> = seen_keys
            .into_iter()
            .take(limit)
            .collect();

        // Determine next cursor for pagination
        let next_cursor = if results.len() == limit {
            // Could be more results; return last key as cursor
            results.last().map(|(k, _)| k.clone())
        } else {
            None
        };

        Ok((results, next_cursor))
    }

    // -------------------------------------------------------------------------
    // Flush
    // -------------------------------------------------------------------------

    fn flush(&self) -> Result<()> {
        // Snapshot the MemTable contents while holding the lock.
        let records: Vec<(String, LogRecord)> = {
            let memtable = self.memtable.lock();
            memtable
                .iter_ordered()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        };

        if records.is_empty() {
            return Ok(());
        }

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = self.dir_path.join(format!("{}.sst", timestamp));

        let mut builder = SstableBuilder::new(path, self.config.storage.clone(), timestamp)?;
        for (key, record) in records {
            builder.add(key.as_bytes(), &record)?;
        }
        let sst_path = builder.finish()?;

        let reader = SstableReader::open(
            sst_path,
            self.config.storage.clone(),
            Arc::clone(&self.block_cache),
        )?;

        // Acquire both locks in a consistent order (sstables write, then
        // memtable) to avoid potential deadlocks with future callers.
        let mut sstables = self.sstables.write();
        let mut memtable = self.memtable.lock();

        sstables.insert(0, reader);
        let cleared = memtable.clear();

        info!(
            "Memtable flushed: {} records, sstables total={}",
            cleared,
            sstables.len()
        );

        drop(memtable);
        drop(sstables);

        self.wal.clear()?;

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Stats
    // -------------------------------------------------------------------------

    pub fn stats(&self) -> String {
        let memtable = self.memtable.lock();
        let sstables = self.sstables.read();
        let cache_stats = self.block_cache.stats();

        format!(
            "LSM Stats:\n MemTable: {} records, ~{} KB\n SSTables: {} files\n Cache: {}/{} blocks",
            memtable.data.len(),
            memtable.size_bytes / 1024,
            sstables.len(),
            cache_stats.len,
            cache_stats.cap
        )
    }

    pub fn stats_all(&self) -> Result<LsmStats> {
        let memtable = self.memtable.lock();
        let sstables = self.sstables.read();

        let mem_records = memtable.data.len();
        let sst_records_total: u64 = sstables.iter().map(|s| s.metadata().record_count).sum();
        let sst_bytes_total: u64 = sstables
            .iter()
            .map(|s| std::fs::metadata(s.path()).map(|m| m.len()).unwrap_or(0))
            .sum();
        let wal_bytes: u64 = std::fs::metadata(&self.wal.path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(LsmStats {
            mem_records,
            mem_kb: memtable.size_bytes / 1024,
            sst_files: sstables.len(),
            sst_records: sst_records_total,
            sst_kb: sst_bytes_total / 1024,
            wal_kb: wal_bytes / 1024,
            total_records: (mem_records as u64) + sst_records_total,
            memtable_max_size: self.config.core.memtable_max_size / 1024,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_engine() -> Result<LsmEngine> {
        let dir = tempdir()?;
        let config = LsmConfig::builder()
            .dir_path(dir.path().to_path_buf())
            .memtable_max_size(4 * 1024) // 4KB for tests
            .build()?;
        Ok(LsmEngine::new(config)?)
    }

    fn setup_test_data(engine: &LsmEngine) {
        // Insert keys in sorted order
        for i in 0..20 {
            let key = format!("user:{:03}", i);
            let value = format!("user_data_{}", i).into_bytes();
            engine.set(key, value).unwrap();
        }
        // Insert some with different prefixes
        engine.set("product:001".to_string(), b"product1".to_vec()).unwrap();
        engine.set("product:002".to_string(), b"product2".to_vec()).unwrap();
    }

    #[test]
    fn test_scan_range_empty_db() -> Result<()> {
        let engine = create_test_engine()?;
        let (results, next_cursor) = engine.scan_range(None, None, 100)?;

        assert!(results.is_empty());
        assert!(next_cursor.is_none());
        Ok(())
    }

    #[test]
    fn test_scan_range_basic() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        let (results, next_cursor) = engine.scan_range(None, None, 100)?;

        assert_eq!(results.len(), 22); // 20 user:* + 2 product:*
        assert!(next_cursor.is_none()); // All results returned

        // Check sorted order
        for i in 1..results.len() {
            assert!(results[i - 1].0 <= results[i].0);
        }
        Ok(())
    }

    #[test]
    fn test_scan_range_with_start() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        let (results, _next_cursor) =
            engine.scan_range(Some("user:010"), None, 100)?;

        // Should start from user:010 (inclusive)
        assert_eq!(results[0].0, "user:010");
        assert_eq!(results.len(), 10); // user:010 to user:019 (10 keys)
        Ok(())
    }

    #[test]
    fn test_scan_range_with_end() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        let (results, _next_cursor) =
            engine.scan_range(None, Some("user:010"), 100)?;

        // Should end before user:010 (exclusive)
        assert!(results.iter().all(|(k, _)| k.as_str() < "user:010"));
        // Contains user:000-009 (10 user keys) + product:001, product:002 (2 product keys) = 12
        assert_eq!(results.len(), 12);
        Ok(())
    }

    #[test]
    fn test_scan_range_with_limit() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        let (results, next_cursor) = engine.scan_range(None, None, 5)?;

        assert_eq!(results.len(), 5);
        assert!(next_cursor.is_some()); // Should have next cursor
        Ok(())
    }

    #[test]
    fn test_scan_range_pagination() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        // First page
        let (page1, cursor) = engine.scan_range(None, None, 5)?;
        assert_eq!(page1.len(), 5);

        // Second page using cursor
        let (page2, _next_cursor) = engine.scan_range(cursor.as_deref(), None, 5)?;
        assert_eq!(page2.len(), 5);

        // Verify no overlap
        let mut keys1: Vec<_> = page1.iter().map(|(k, _)| k).collect();
        let keys2: Vec<_> = page2.iter().map(|(k, _)| k).collect();

        // First page ends before second page starts
        for (key1, key2) in keys1.iter().zip(keys2.iter()) {
            assert!(key1 < key2);
        }
        Ok(())
    }

    #[test]
    fn test_scan_range_invalid_args() -> Result<()> {
        let engine = create_test_engine()?;

        // limit = 0
        assert!(engine.scan_range(None, None, 0).is_err());

        // limit > max
        assert!(engine.scan_range(None, None, 20000).is_err());

        // start >= end
        assert!(engine.scan_range(Some("b"), Some("a"), 100).is_err());
        assert!(engine.scan_range(Some("a"), Some("a"), 100).is_err());

        Ok(())
    }

    #[test]
    fn test_scan_range_tombstones() -> Result<()> {
        let engine = create_test_engine()?;

        // Insert and delete
        engine.set("user:001".to_string(), b"original".to_vec())?;
        engine.set("user:002".to_string(), b"to_be_deleted".to_vec())?;
        engine.delete("user:002".to_string())?;
        engine.set("user:003".to_string(), b"final".to_vec())?;

        let (results, _next_cursor) = engine.scan_range(None, None, 100)?;

        // Should have user:001 and user:003, but not user:002 (tombstone)
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(k, _)| k == "user:001"));
        assert!(results.iter().any(|(k, _)| k == "user:003"));
        assert!(!results.iter().any(|(k, _)| k == "user:002"));
        Ok(())
    }

    #[test]
    fn test_scan_range_memtable_overrides_sstable() -> Result<()> {
        let dir = tempdir()?;
        let config = LsmConfig::builder()
            .dir_path(dir.path().to_path_buf())
            .memtable_max_size(4 * 1024)
            .build()?;
        let engine = LsmEngine::new(config)?;

        // Insert in memtable
        engine.set("user:001".to_string(), b"memtable_value".to_vec())?;

        // Force flush to sstable
        engine.flush()?;

        // Update in memtable - this should override sstable value
        engine.set("user:001".to_string(), b"new_memtable_value".to_vec())?;

        let (results, _next_cursor) = engine.scan_range(None, None, 100)?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1, b"new_memtable_value");
        Ok(())
    }

    #[test]
    fn test_search_prefix_empty_db() -> Result<()> {
        let engine = create_test_engine()?;
        let (results, next_cursor) = engine.search_prefix("user:", None, 100)?;

        assert!(results.is_empty());
        assert!(next_cursor.is_none());
        Ok(())
    }

    #[test]
    fn test_search_prefix_basic() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        let (results, next_cursor) = engine.search_prefix("user:", None, 100)?;

        assert_eq!(results.len(), 20);
        assert!(next_cursor.is_none());
        assert!(results.iter().all(|(k, _)| k.starts_with("user:")));
        Ok(())
    }

    #[test]
    fn test_search_prefix_pagination() -> Result<()> {
        let engine = create_test_engine()?;
        setup_test_data(&engine);

        // First page
        let (page1, cursor) = engine.search_prefix("user:", None, 5)?;
        assert_eq!(page1.len(), 5);

        // Second page using cursor
        let (page2, _next_cursor) =
            engine.search_prefix("user:", cursor.as_deref(), 5)?;
        assert_eq!(page2.len(), 5);

        // Keys in second page should all be after cursor
        if let Some(cur_str) = &cursor {
            let cur = cur_str.as_str();
            for (k, _) in &page2 {
                assert!(cur < k.as_str());
            }
        }
        Ok(())
    }

    #[test]
    fn test_search_prefix_invalid_args() -> Result<()> {
        let engine = create_test_engine()?;

        // limit = 0
        assert!(engine.search_prefix("user:", None, 0).is_err());

        // limit > max
        assert!(engine.search_prefix("user:", None, 20000).is_err());

        Ok(())
    }

    #[test]
    fn test_search_prefix_tombstones() -> Result<()> {
        let engine = create_test_engine()?;

        // Insert and delete
        engine.set("user:001".to_string(), b"original".to_vec())?;
        engine.set("user:002".to_string(), b"to_be_deleted".to_vec())?;
        engine.delete("user:002".to_string())?;
        engine.set("user:003".to_string(), b"final".to_vec())?;

        let (results, _next_cursor) =
            engine.search_prefix("user:", None, 100)?;

        // Should have user:001 and user:003, but not user:002 (tombstone)
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(k, _)| k == "user:001"));
        assert!(results.iter().any(|(k, _)| k == "user:003"));
        assert!(!results.iter().any(|(k, _)| k == "user:002"));
        Ok(())
    }

    // Performance test
    #[test]
    #[ignore] // Run with `cargo test -- --ignored` for performance tests
    fn test_scan_range_performance_100k_keys() -> Result<()> {
        let engine = create_test_engine()?;

        // Insert 100k keys
        for i in 0..100_000 {
            let key = format!("perf:{}", i);
            let value = vec![b'x'; 64];
            engine.set(key, value).unwrap();
        }

        // Force flush
        engine.flush()?;

        // Measure scan performance
        let start = std::time::Instant::now();
        let (results, _cursor) = engine.scan_range(None, None, 10)?;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 10);
        assert!(elapsed.as_millis() < 50, "Scan should return quickly, took {:?}", elapsed);

        Ok(())
    }
}
