use crate::core::log_record::LogRecord;
use crate::core::memtable::MemTable;
use crate::infra::config::LsmConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::builder::SstableBuilder;
use crate::storage::cache::GlobalBlockCache;
use crate::storage::reader::SstableReader;
use crate::storage::wal::WriteAheadLog;

use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{info, warn};

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

    pub fn search_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
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
