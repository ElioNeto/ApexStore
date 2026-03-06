use crate::core::log_record::LogRecord;
use crate::core::memtable::MemTable;
use crate::infra::config::LsmConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::builder::SstableBuilder;
use crate::storage::cache::GlobalBlockCache;
use crate::storage::reader::SstableReader;
use crate::storage::wal::WriteAheadLog;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
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

pub struct LsmEngine {
    pub(crate) memtable: Mutex<MemTable>,
    pub(crate) wal: WriteAheadLog,
    pub(crate) sstables: Mutex<Vec<SstableReader>>,
    pub(crate) block_cache: Arc<GlobalBlockCache>,
    pub(crate) dir_path: PathBuf,
    pub(crate) config: LsmConfig,
}

impl LsmEngine {
    pub fn new(config: LsmConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.core.dir_path)?;

        // Create global shared block cache
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

        // Sort by timestamp descending (newest first)
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
            sstables: Mutex::new(sstables),
            block_cache,
            dir_path: config.core.dir_path.clone(),
            config,
        })
    }

    fn memtable_lock(&self) -> Result<MutexGuard<'_, MemTable>> {
        self.memtable
            .lock()
            .map_err(|_| LsmError::LockPoisoned("memtable"))
    }

    fn sstables_lock(&self) -> Result<MutexGuard<'_, Vec<SstableReader>>> {
        self.sstables
            .lock()
            .map_err(|_| LsmError::LockPoisoned("sstables"))
    }

    pub fn set(&self, key: String, value: Vec<u8>) -> Result<()> {
        let record = LogRecord::new(key, value);
        self.wal.write_record(&record)?;

        let mut memtable = self.memtable_lock()?;
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

        let mut memtable = self.memtable_lock()?;
        memtable.insert(record);

        if memtable.should_flush() {
            drop(memtable);
            self.flush()?;
        }

        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let memtable = self.memtable_lock()?;
        if let Some(record) = memtable.get(key) {
            return Ok(if record.is_deleted {
                None
            } else {
                Some(record.value)
            });
        }
        drop(memtable);

        // 2. Check SSTables (newest to oldest)
        let mut sstables = self.sstables_lock()?;
        for sst in sstables.iter_mut() {
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

    pub fn set_batch(&self, items: Vec<(String, Vec<u8>)>) -> Result<usize> {
        let mut count = 0;
        for (key, value) in items {
            self.set(key, value)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn delete_batch(&self, keys: Vec<String>) -> Result<usize> {
        let mut count = 0;
        for key in keys {
            self.delete(key)?;
            count += 1;
        }
        Ok(count)
    }

    pub fn search(&self, pattern: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let all_data = self.scan()?;
        Ok(all_data
            .into_iter()
            .filter(|(key, _)| key.contains(pattern))
            .collect())
    }

    pub fn search_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let all_data = self.scan()?;
        Ok(all_data
            .into_iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .collect())
    }

    fn flush(&self) -> Result<()> {
        let mut memtable = self.memtable_lock()?;
        let records: Vec<(String, LogRecord)> = memtable
            .iter_ordered()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if records.is_empty() {
            return Ok(());
        }

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let filename = format!("{}.sst", timestamp);
        let path = self.dir_path.join(filename);

        // Create new SSTable using Builder (V2)
        let mut builder = SstableBuilder::new(path, self.config.storage.clone(), timestamp)?;
        for (key, record) in records {
            builder.add(key.as_bytes(), &record)?;
        }
        let sst_path = builder.finish()?;

        // Open the new SSTable as Reader (V2) with shared cache
        let reader = SstableReader::open(
            sst_path,
            self.config.storage.clone(),
            Arc::clone(&self.block_cache),
        )?;

        let mut sstables = self.sstables_lock()?;
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

    pub fn scan(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let mut result_map: HashMap<String, (Vec<u8>, u128, bool)> = HashMap::new();

        let memtable = self.memtable_lock()?;
        for (key, record) in memtable.iter_ordered() {
            result_map.insert(
                key.clone(),
                (record.value.clone(), record.timestamp, record.is_deleted),
            );
        }
        drop(memtable);

        let mut sstables = self.sstables_lock()?;
        for sst in sstables.iter_mut() {
            let records = sst.scan()?;
            for (key_bytes, record) in records {
                let key = String::from_utf8(key_bytes)
                    .map_err(|e| LsmError::CorruptedData(e.to_string()))?;
                result_map.entry(key).or_insert((
                    record.value,
                    record.timestamp,
                    record.is_deleted,
                ));
            }
        }
        drop(sstables);

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
        let all_data = self.scan()?;
        Ok(all_data.into_iter().map(|(k, _)| k).collect())
    }

    pub fn count(&self) -> Result<usize> {
        Ok(self.scan()?.len())
    }

    pub fn stats(&self) -> String {
        let memtable = match self.memtable_lock() {
            Ok(g) => g,
            Err(e) => return format!("LSM Stats error: {e}"),
        };
        let sstables = match self.sstables_lock() {
            Ok(g) => g,
            Err(e) => return format!("LSM Stats error: {e}"),
        };

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

    pub fn stats_all(&self) -> std::result::Result<LsmStats, String> {
        let memtable = self.memtable_lock().map_err(|e| e.to_string())?;
        let sstables = self.sstables_lock().map_err(|e| e.to_string())?;

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
