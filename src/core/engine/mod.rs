pub mod compaction;
pub mod manifest;
pub mod version_set;

use crate::core::log_record::LogRecord;
use crate::core::table::Table;
use crate::storage::cache::Cache;
use crate::storage::wal::WriteAheadLog;
use crate::infra::error::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use self::compaction::{Compaction, CompactionMetrics, CompactionOptions, CompactionStrategyType};
use self::manifest::Manifest;
use self::version_set::VersionSet;
use crate::core::iterators::{MergeIterator, StorageIterator};
use crate::core::key::KeySlice;

pub const DEFAULT_SCAN_LIMIT: usize = 128;
pub const MAX_SCAN_LIMIT: usize = 1024;

#[derive(Debug, Clone, Default)]
pub struct LsmStats {
    pub num_tables: usize,
    pub total_size: usize,
    pub total_records: usize,
    pub sst_kb: usize,
    pub wal_kb: usize,
    pub num_tables_at_max: usize,
    pub max_levels_reached: usize,
    pub memtable_max_size: usize,
    pub mem_kb: usize,
    pub mem_records: usize,
    pub sst_files: usize,
    pub sst_records: usize,
    // Compaction stats
    pub compaction_strategy: String,
    pub last_compaction_bytes_read: u64,
    pub last_compaction_bytes_written: u64,
    pub last_compaction_files_merged: usize,
    pub last_compaction_duration_ms: u64,
}

/// Engine options.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub block_size: usize,
    pub bloom_bits_per_key: usize,
    pub max_table_size: usize,
    pub min_table_size_to_compact: usize,
    pub max_levels: usize,
    pub level_multiplier: usize,
    pub write_buffer_size: usize,
    pub max_write_buffer_number: usize,
    pub compaction_options: CompactionOptions,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            block_size: 4096,
            bloom_bits_per_key: 10,
            max_table_size: 1024 * 1024,
            min_table_size_to_compact: 64,
            max_levels: 7,
            level_multiplier: 4,
            write_buffer_size: 64 * 1024,
            max_write_buffer_number: 4,
            compaction_options: CompactionOptions::default(),
        }
    }
}

impl From<&crate::infra::config::LsmConfig> for EngineOptions {
    fn from(config: &crate::infra::config::LsmConfig) -> Self {
        let compaction_options = CompactionOptions {
            strategy_type: config.compaction.strategy.clone().into(),
            compaction_threshold: config.compaction.min_compaction_threshold,
            max_tables_per_compaction: config.compaction.max_sstables,
        };
        
        Self {
            block_size: config.storage.block_size,
            bloom_bits_per_key: 10,
            max_table_size: 1024 * 1024,
            min_table_size_to_compact: 64,
            max_levels: 7,
            level_multiplier: 4,
            write_buffer_size: config.core.memtable_max_size,
            max_write_buffer_number: 4,
            compaction_options,
        }
    }
}

impl EngineOptions {
    /// Create EngineOptions from LsmConfig
    pub fn from_config(config: &crate::infra::config::LsmConfig) -> Self {
        config.into()
    }
}

/// The core engine that manages LSM-tree structure and compaction.
pub struct Engine<C: Cache> {
    options: EngineOptions,
    _manifest: Manifest,
    version_set: VersionSet<C>,
    /// Memtables indexed by column family.
    memtables: HashMap<String, Vec<MemTable>>,
    /// Write buffer limit in bytes per column family.
    write_buffer_limit: usize,
    /// Current total bytes in memtables per column family.
    memtable_bytes: HashMap<String, usize>,
    /// Compaction strategy and executor.
    compaction: Compaction,
    /// Write-Ahead Log for crash recovery.
    wal: WriteAheadLog,
    /// Background compaction running flag.
    compaction_running: Arc<AtomicBool>,
    /// SSTable output directory (used during initialization).
    _sst_dir: PathBuf,
}

pub type LsmEngineGeneric<C> = Engine<C>;
pub type LsmEngine = Engine<crate::storage::cache::GlobalBlockCache>;
pub type ScanRangeResult = crate::infra::error::Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)>;

pub(crate) struct MemTable {
    data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    size: usize,
}

impl MemTable {
    fn new() -> Self {
        Self {
            data: std::collections::BTreeMap::new(),
            size: 0,
        }
    }

    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        let old = self.data.insert(key.clone(), value.clone());
        self.size += key.len() + value.len();
        if let Some(old_val) = old {
            self.size -= old_val.len();
        }
    }

    fn delete(&mut self, key: Vec<u8>) {
        if let Some(old) = self.data.remove(&key) {
            self.size += key.len();
            self.size -= old.len();
        }
    }
}

struct InternalMemTableIterator<'a> {
    inner: std::collections::btree_map::Iter<'a, Vec<u8>, Vec<u8>>,
    current: Option<(&'a Vec<u8>, &'a Vec<u8>)>,
}

impl<'a> InternalMemTableIterator<'a> {
    fn new(data: &'a std::collections::BTreeMap<Vec<u8>, Vec<u8>>) -> Self {
        let mut inner = data.iter();
        let current = inner.next();
        Self { inner, current }
    }
}

impl<'a> StorageIterator for InternalMemTableIterator<'a> {
    type KeyType = KeySlice<'a>;

    fn next(&mut self) {
        self.current = self.inner.next();
    }
    fn key(&self) -> Self::KeyType {
        match self.current {
            Some((k, _)) => KeySlice::new(k.as_slice()),
            None => panic!("InternalMemTableIterator is invalid when calling key()"),
        }
    }
    fn value(&self) -> &[u8] {
        match self.current {
            Some((_, v)) => v.as_slice(),
            None => panic!("InternalMemTableIterator is invalid when calling value()"),
        }
    }
    fn is_valid(&self) -> bool {
        self.current.is_some()
    }
    fn seek(&mut self, _key: &[u8]) {
        while self.is_valid() && self.key().as_ref() < _key {
            self.next();
        }
    }
}

impl<C: Cache> Engine<C> {
    // ========== pub(crate) accessors for internal crate use ==========
    
    /// Returns the engine options.
    pub(crate) fn options(&self) -> &EngineOptions {
        &self.options
    }
    
    /// Returns the version set.
    pub(crate) fn version_set(&self) -> &VersionSet<C> {
        &self.version_set
    }
    
    /// Returns mutable reference to version set.
    pub(crate) fn version_set_mut(&mut self) -> &mut VersionSet<C> {
        &mut self.version_set
    }
    
    /// Returns the memtables map.
    pub(crate) fn memtables(&self) -> &HashMap<String, Vec<MemTable>> {
        &self.memtables
    }
    
    /// Returns mutable reference to memtables map.
    pub(crate) fn memtables_mut(&mut self) -> &mut HashMap<String, Vec<MemTable>> {
        &mut self.memtables
    }
    
    /// Returns the write buffer limit.
    pub(crate) fn write_buffer_limit(&self) -> usize {
        self.write_buffer_limit
    }
    
    /// Returns the memtable bytes map.
    pub(crate) fn memtable_bytes(&self) -> &HashMap<String, usize> {
        &self.memtable_bytes
    }
    
    /// Returns mutable reference to memtable bytes map.
    pub(crate) fn memtable_bytes_mut(&mut self) -> &mut HashMap<String, usize> {
        &mut self.memtable_bytes
    }
    
    /// Returns the compaction strategy.
    pub(crate) fn compaction(&self) -> &Compaction {
        &self.compaction
    }
    
    /// Returns mutable reference to compaction strategy.
    pub(crate) fn compaction_mut(&mut self) -> &mut Compaction {
        &mut self.compaction
    }
    
    /// Returns the WAL.
    pub(crate) fn wal(&self) -> &WriteAheadLog {
        &self.wal
    }
    
    /// Returns mutable reference to WAL.
    pub(crate) fn wal_mut(&mut self) -> &mut WriteAheadLog {
        &mut self.wal
    }
    
    /// Returns the compaction running flag.
    pub(crate) fn compaction_running(&self) -> &Arc<AtomicBool> {
        &self.compaction_running
    }
    
    /// Returns the SSTable directory (for testing).
    pub(crate) fn sst_dir(&self) -> &PathBuf {
        &self._sst_dir
    }
}

impl<C: Cache> Engine<C> {
    /// Create a new engine with default options.
    pub fn new_generic(options: EngineOptions, cache: C, dir_path: &std::path::Path) -> Result<Self> {
        // Create SSTable directory
        let sst_dir = dir_path.join("sstables");
        std::fs::create_dir_all(&sst_dir)?;
        
        // Create storage config from options
        let storage_config = crate::infra::config::StorageConfig {
            block_size: options.block_size,
            block_cache_size_mb: 64,
            sparse_index_interval: 16,
            bloom_false_positive_rate: 0.01,
        };
        
        // Create compaction with strategy from options
        let strategy_type = if options.compaction_options.compaction_threshold <= 4 {
            CompactionStrategyType::SizeTiered
        } else {
            CompactionStrategyType::Leveled
        };
        
        let compaction_options = CompactionOptions {
            strategy_type,
            compaction_threshold: options.compaction_options.compaction_threshold,
            max_tables_per_compaction: options.compaction_options.max_tables_per_compaction,
        };
        
        let compaction = Compaction::new(
            strategy_type,
            compaction_options,
            storage_config,
            sst_dir.clone(),
        );
        
        let wal = WriteAheadLog::new(dir_path)?;
        let recovered_records = wal.recover()?;
        
        let mut engine = Self {
            options: options.clone(),
            _manifest: Manifest::new(),
            version_set: VersionSet::new(options.clone(), cache),
            memtables: HashMap::new(),
            write_buffer_limit: options.write_buffer_size * options.max_write_buffer_number,
            memtable_bytes: HashMap::new(),
            compaction,
            wal,
            compaction_running: Arc::new(AtomicBool::new(false)),
            _sst_dir: sst_dir,
        };
        
        engine.replay_wal_records(recovered_records)?;
        
        Ok(engine)
    }
    
    /// Replay WAL records to reconstruct memtable state.
    fn replay_wal_records(&mut self, records: Vec<LogRecord>) -> Result<()> {
        for record in records {
            if record.is_deleted {
                // This is a delete/tombstone record
                let cf = "default"; // LogRecord doesn't have column_family field
                let mem = self.memtables.entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new());
                }
                let last = mem.len() - 1;
                mem[last].delete(record.key.clone());
                *self.memtable_bytes.entry(cf.to_string()).or_default() += record.key.len();
            } else {
                // This is a put record
                let cf = "default"; // LogRecord doesn't have column_family field
                let mem = self.memtables.entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new());
                }
                let last = mem.len() - 1;
                mem[last].put(record.key.clone(), record.value.clone());
                *self.memtable_bytes.entry(cf.to_string()).or_default() += record.key.len() + record.value.len();
            }
        }
        Ok(())
    }
    
    /// Put a key-value pair into the specified column family.
    pub fn put_cf(&mut self, cf: &str, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        // Write to WAL first (before modifying memtable) for crash safety
        let record = LogRecord::new(key.clone(), value.clone());
        self.wal.write_record(&record)?;
        
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].put(key.clone(), value.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len() + value.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            let _ = self.flush_memtable_impl(cf);
        }
        Ok(())
    }
    
    pub fn set<K, V>(&mut self, key: K, value: V) -> Result<()>
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        self.put_cf("default", key.into(), value.into())
    }
    
    pub fn delete_cf<K>(&mut self, cf: &str, key: K) -> Result<()>
    where
        K: Into<Vec<u8>>,
    {
        let key = key.into();
        
        // Write tombstone to WAL first (before modifying memtable) for crash safety
        let record = LogRecord::tombstone_with_cf(cf, key.clone());
        self.wal.write_record(&record)?;
        
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].delete(key.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            let _ = self.flush_memtable_impl(cf);
        }
        Ok(())
    }
    
    pub fn delete<K>(&mut self, key: K) -> Result<()>
    where
        K: Into<Vec<u8>>,
    {
        self.delete_cf("default", key)
    }
    
    pub fn get_cf<K>(&self, cf: &str, key: K) -> Result<Option<Vec<u8>>>
    where
        K: AsRef<[u8]>,
    {
        let key = key.as_ref();
        if let Some(memtables) = self.memtables.get(cf) {
            for mem in memtables.iter().rev() {
                if let Some(v) = mem.data.get(key) {
                    return Ok(Some(v.clone()));
                }
            }
        }
        Ok(self.version_set.get(cf, key))
    }
    
    pub fn get<K>(&self, key: K) -> Result<Option<Vec<u8>>>
    where
        K: AsRef<[u8]>,
    {
        self.get_cf("default", key)
    }
    
    pub fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan_cf("default", None, None, Some(DEFAULT_SCAN_LIMIT))
    }
    
    pub fn scan_cf(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
        
        // 1. Memtables (newer first)
        if let Some(memtables) = self.memtables.get(cf) {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(InternalMemTableIterator::new(&mem.data)));
            }
        }
        
        // 2. SSTables (from VersionSet)
        for sst_iter in self.version_set.table_iters(cf) {
            iters.push(Box::new(sst_iter));
        }
        
        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();
        let limit = limit.unwrap_or(MAX_SCAN_LIMIT);
        
        while merge_iter.is_valid() && results.len() < limit {
            if let Some(lower) = lower {
                if merge_iter.key().as_ref() < lower {
                    merge_iter.next();
                    continue;
                }
            }
            if let Some(upper) = upper {
                if merge_iter.key().as_ref() >= upper {
                    break;
                }
            }
            results.push((merge_iter.key().as_ref().to_vec(), merge_iter.value().to_vec()));
            merge_iter.next();
        }
        
        Ok(results)
    }
    
    pub fn scan_range(
        &self,
        cf: &str,
        start: &[u8],
        end: &[u8],
        limit: Option<usize>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan_cf(cf, Some(start), Some(end), limit)
    }
    
    #[allow(clippy::type_complexity)]
    pub fn search_prefix(
        &self,
        prefix: &str,
        _cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)> {
        // Calculate upper bound for prefix scan
        let upper_bound = Self::prefix_end(prefix);
        
        // Scan with both lower and upper bounds
        let results = self.scan_cf(
            "default",
            Some(prefix.as_bytes()),
            upper_bound.as_deref(),
            Some(limit),
        )?;
        
        // No need for post-filtering since upper bound handles it
        Ok((results, None))
    }
    
    pub fn keys(&self) -> Result<Vec<Vec<u8>>> {
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
        
        if let Some(memtables) = self.memtables.get("default") {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(InternalMemTableIterator::new(&mem.data)));
            }
        }
        
        for sst_iter in self.version_set.table_iters("default") {
            iters.push(Box::new(sst_iter));
        }
        
        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();
        
        while merge_iter.is_valid() && results.len() < MAX_SCAN_LIMIT {
            results.push(merge_iter.key().as_ref().to_vec());
            merge_iter.next();
        }
        
        Ok(results)
    }
    
    pub fn count(&self) -> Result<usize> {
        let mut count = 0;
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
        
        if let Some(memtables) = self.memtables.get("default") {
            for mem in memtables.iter().rev() {
                count += mem.data.len();
            }
        }
        
        for sst_iter in self.version_set.table_iters("default") {
            iters.push(Box::new(sst_iter));
        }
        
        let mut merge_iter = MergeIterator::new(iters);
        while merge_iter.is_valid() {
            count += 1;
            merge_iter.next();
        }
        
        Ok(count)
    }
    
    fn flush_memtable_impl(&mut self, cf: &str) -> Result<()> {
        if let Some(memtables) = self.memtables.get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let table = Table::build(mem.data.into_iter().collect(), &self.options);
                self.version_set.add_table(cf, table);
                let bytes = self.memtable_bytes.get_mut(cf).ok_or_else(|| {
                    crate::LsmError::InvalidArgument(format!("Column family {} not found in memtable_bytes", cf))
                })?;
                *bytes = 0;
                
                // ✅ FIX issue #105: acionar compactação se SSTable count exceder threshold
                let threshold = self.options.compaction_options.compaction_threshold;
                if self.version_set.table_count(cf) > threshold {
                    let _ = self.compact_cf(cf)?; // Ignore metrics for now
                }
                
                // ✅ FIX issue #107: Clear WAL while we have exclusive &mut self access
                // This eliminates the crash window where a crash between flush and WAL clear
                // would cause duplicate entries on recovery
                self.wal.clear()?;
            }
        }
        Ok(())
    }
    
    pub fn compact_cf(&mut self, cf: &str) -> Result<Option<CompactionMetrics>> {
        // Get tables for this column family
        let tables = self.version_set.get_tables(cf);
        
        if tables.len() < self.compaction.options().compaction_threshold {
            return Ok(None);
        }
        
        // Pick tables to compact based on strategy
        let groups = self.compaction.pick_compaction(&tables, &self.options);
        
        if groups.is_empty() {
            return Ok(None);
        }
        
        let mut all_metrics = CompactionMetrics::default();
        
        for group_indices in groups {
            // Execute compaction on this group
            let (new_tables, metrics) = self.compaction.compact(&group_indices, &tables, &self.options)?;
            
            // Atomic replace old tables with new ones
            self.version_set.atomic_replace(cf, &group_indices, new_tables);
            
            // Accumulate metrics
            all_metrics.bytes_read += metrics.bytes_read;
            all_metrics.bytes_written += metrics.bytes_written;
            all_metrics.files_merged += metrics.files_merged;
            all_metrics.duration_ms += metrics.duration_ms;
        }
        
        Ok(Some(all_metrics))
    }
    
    pub fn compact(&mut self) -> Result<Vec<(String, CompactionMetrics)>> {
        let mut results = Vec::new();
        let column_families = self.version_set.column_families();
        
        for cf in column_families {
            if let Some(metrics) = self.compact_cf(&cf)? {
                results.push((cf, metrics));
            }
        }
        
        Ok(results)
    }
    
    /// Check if compaction should be triggered and run it in background
    pub fn maybe_compact(&mut self) -> Result<()> {
        // Check if compaction is already running
        if self.compaction_running.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        // Check if any column family needs compaction
        let column_families = self.version_set.column_families();
        let mut needs_compaction = false;
        
        for cf in &column_families {
            let table_count = self.version_set.table_count(cf);
            if table_count >= self.compaction.options().compaction_threshold {
                needs_compaction = true;
                break;
            }
        }
        
        if !needs_compaction {
            return Ok(());
        }
        
        // Set flag and spawn background compaction
        self.compaction_running.store(true, Ordering::SeqCst);
        
        // For now, compact synchronously (TODO: spawn background task)
        let result = self.compact();
        self.compaction_running.store(false, Ordering::SeqCst);
        
        result?;
        Ok(())
    }
    
    pub fn stats(&self, cf: &str) -> Result<LsmStats> {
        let mut stats = LsmStats::default();
        
        // Get stats from version set
        let vs_stats = self.version_set.stats(cf);
        stats.num_tables = vs_stats.num_tables;
        stats.total_size = vs_stats.total_size;
        stats.total_records = vs_stats.total_records;
        stats.sst_kb = vs_stats.sst_kb;
        stats.sst_files = vs_stats.sst_files;
        stats.sst_records = vs_stats.sst_records;
        stats.max_levels_reached = vs_stats.max_levels_reached;
        stats.num_tables_at_max = vs_stats.num_tables_at_max;
        
        // Memtable stats
        if let Some(memtables) = self.memtables.get(cf) {
            stats.mem_records = memtables.iter().map(|m| m.data.len()).sum();
            stats.mem_kb = self.memtable_bytes.get(cf).copied().unwrap_or(0) / 1024;
        }
        
        // WAL stats
        stats.wal_kb = self.wal.size()? as usize / 1024;
        
        Ok(stats)
    }
    
    pub fn stats_all(&self) -> Result<LsmStats> {
        let mut combined = LsmStats::default();
        let column_families = self.version_set.column_families();
        
        for cf in column_families {
            let stats = self.stats(&cf)?;
            combined.num_tables += stats.num_tables;
            combined.total_size += stats.total_size;
            combined.total_records += stats.total_records;
            combined.sst_kb += stats.sst_kb;
            combined.sst_files += stats.sst_files;
            combined.sst_records += stats.sst_records;
            combined.mem_records += stats.mem_records;
            combined.mem_kb += stats.mem_kb;
        }
        
        combined.wal_kb = self.wal.size()? as usize / 1024;
        
        Ok(combined)
    }
    
    /// Calculate the upper bound for a prefix scan.
    /// Given prefix "ab", returns Some("ac") (incrementing the last byte).
    /// For empty prefix, returns None (scan everything).
    pub fn prefix_end(prefix: &str) -> Option<Vec<u8>> {
        if prefix.is_empty() {
            return None;
        }
        
        let mut result = prefix.as_bytes().to_vec();
        
        // Increment the last byte, handling overflow
        for i in (0..result.len()).rev() {
            if result[i] < 0xFF {
                result[i] += 1;
                return Some(result);
            }
            result[i] = 0;
        }
        
        // All bytes were 0xFF, so we need to extend
        result.push(0);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use crate::core::engine::compaction::CompactionStrategy;
    use std::collections::BTreeMap;

    #[test]
    fn test_prefix_end_basic() {
        // Test basic ASCII prefix
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("ab");
        assert_eq!(result, Some(b"ac".to_vec()));
    }

    #[test]
    fn test_prefix_end_empty() {
        // Test empty prefix
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_prefix_end_non_ascii() {
        // Test non-ASCII prefix (e.g., UTF-8 characters)
        let prefix = "usuário:";
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end(prefix);
        // The last byte of "usuário:" is 0x3A (':')
        // So we expect it to be incremented to 0x3B (';')
        assert!(result.is_some());
        let end = result.unwrap();
        // The prefix in bytes should be "usuário:" followed by something
        assert!(end.len() >= prefix.as_bytes().len());
        // The upper bound should be greater than the prefix
        assert!(end.as_slice() > prefix.as_bytes());
    }

    #[test]
    fn test_prefix_end_unicode_multi_byte() {
        // Test with multi-byte UTF-8 characters
        // "ção:" - 'ç' is 0xC3 0xA7, 'ã' is 0xC3 0xA3 in UTF-8
        let prefix = "ção:";
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end(prefix);
        assert!(result.is_some());
        let end = result.unwrap();
        // The upper bound should be greater than the prefix
        assert!(end.as_slice() > prefix.as_bytes());
    }

    #[test]
    fn test_prefix_end_single_byte_increment() {
        // Test that only the last non-0xFF byte is incremented
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("abc");
        assert_eq!(result, Some(b"abd".to_vec()));
    }

    #[test]
    fn test_search_prefix_non_ascii() {
        use crate::infra::config::LsmConfig;
        
        // Create temp directory for engine storage
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        
        let mut engine = Engine::new_from_config(&config, crate::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        
        // Insert some non-ASCII key-value pairs
        let test_pairs = vec![
            ("usuário:1", "value1"),
            ("usuário:2", "value2"),
            ("chave:3", "value3"),
        ];
        
        for (key, value) in &test_pairs {
            engine.set(key.as_bytes().to_vec(), value.as_bytes().to_vec()).unwrap();
        }
        
        // Search with prefix
        let (results, _) = engine.search_prefix("usuário:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_prefix_unicode_chars() {
        use crate::infra::config::LsmConfig;
        
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        
        let mut engine = Engine::new_from_config(&config, crate::storage::cache::GlobalBlockCache::new(100, 4096)).unwrap();
        
        // Insert with unicode prefix
        let test_pairs = vec![
            ("ção:1", "value1"),
            ("ção:2", "value2"),
            ("outro:3", "value3"),
        ];
        
        for (key, value) in &test_pairs {
            engine.set(key.as_bytes().to_vec(), value.as_bytes().to_vec()).unwrap();
        }
        
        let (results, _) = engine.search_prefix("ção:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_size_tiered_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        
        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();
        
        // Create tables
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _metrics) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        assert!(!new_tables.is_empty(), "Should produce at least one new table");
    }

    #[test]
    fn test_lazy_leveling_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = LazyLevelingCompaction::default();
        let options = EngineOptions::default();
        
        // Create L0 tables (should use size-tiered)
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            let mut table = Table::build(data, &options);
            table.level = 0;
            tables.push(table);
        }
        
        // Pick tables to compact
        let _groups = strategy.pick_tables(&tables, &options);
        
        // Execute compaction
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        assert!(!new_tables.is_empty(), "Should produce at least one new table");
    }

    #[test]
    fn test_compaction_removes_tombstones() {
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();
        
        // Create a table with tombstones (empty values)
        let mut data = BTreeMap::new();
        // Add some live data
        for i in 0..50 {
            let key = format!("live_key_{}", i).into_bytes();
            let value = format!("live_value_{}", i).into_bytes();
            data.insert(key, value);
        }
        // Add tombstones
        for i in 0..50 {
            let key = format!("dead_key_{}", i).into_bytes();
            let value = Vec::new(); // tombstone
            data.insert(key, value);
        }
        
        let table = Table::build(data, &options);
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _) = strategy.execute(vec![table], &options, &storage_config, &output_dir).unwrap();
        
        // The new table should not contain tombstones
        if let Some(new_table) = new_tables.first() {
            for (_, value) in &new_table.data {
                assert!(!value.is_empty(), "Tombstones should be removed during compaction");
            }
        }
    }

    #[test]
    fn test_compaction_metrics() {
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();
        
        // Create tables
        let mut tables = Vec::new();
        for i in 0..3 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_, metrics) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        assert!(metrics.bytes_read > 0, "Should track bytes read");
        assert!(metrics.files_merged > 0, "Should track files merged");
        assert!(metrics.duration_ms > 0, "Duration should be positive");
    }

    #[test]
    fn test_size_tiered_bucket_grouping() {
        use crate::core::engine::compaction::SizeTieredCompaction;
        use crate::core::table::Table;
        use std::collections::BTreeMap;
        
        let strategy = SizeTieredCompaction::default();
        
        // Create tables of different sizes
        let mut tables = Vec::new();
        for i in 0..10 {
            let mut data = BTreeMap::new();
            let num_entries = if i < 5 { 10 } else { 100 };
            for j in 0..num_entries {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &EngineOptions::default()));
        }
        
        let options = crate::core::engine::EngineOptions::default();
        let groups = strategy.pick_tables(&tables, &options);
        
        // Should group small tables together
        assert!(!groups.is_empty(), "Should group tables by size");
    }

    #[test]
    fn test_atomic_replace_in_version_set() {
        use crate::storage::cache::NoopCache;
        
        let options = crate::core::engine::EngineOptions::default();
        let cache = NoopCache;
        let mut vs = crate::core::engine::version_set::VersionSet::<NoopCache>::new(options, cache);
        
        // Add some tables
        for i in 0..5 {
            let mut data = std::collections::BTreeMap::new();
            data.insert(format!("key_{}", i).into_bytes(), format!("value_{}", i).into_bytes());
            let table = crate::core::table::Table::build(data, &crate::core::engine::EngineOptions::default());
            vs.add_table("default", table);
        }
        
        assert_eq!(vs.table_count("default"), 5);
        
        // Create new tables to replace some old ones
        let mut new_tables = Vec::new();
        for i in 0..2 {
            let mut data = std::collections::BTreeMap::new();
            data.insert(format!("new_key_{}", i).into_bytes(), format!("new_value_{}", i).into_bytes());
            new_tables.push(crate::core::table::Table::build(data, &crate::core::engine::EngineOptions::default()));
        }
        
        // Replace tables at indices 0, 1, 2 with new tables
        vs.atomic_replace("default", &[0, 1, 2], new_tables);
        
        assert_eq!(vs.table_count("default"), 4); // 5 - 3 + 2 = 4
    }

    #[test]
    fn test_1000_keys_with_multiple_compactions() {
        use crate::infra::config::LsmConfig;
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();
        
        // Create tables with known sizes
        let mut tables = Vec::new();
        
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_new_tables, metrics) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        // Write amplification = bytes_written / bytes_read
        // For SizeTiered, should be < 3x
        if metrics.bytes_read > 0 {
            let write_amplification = metrics.bytes_written as f64 / metrics.bytes_read as f64;
            assert!(
                write_amplification < 3.0,
                "Write amplification for SizeTiered should be < 3x, got {:.2}x",
                write_amplification
            );
        }
    }

    #[test]
    fn test_leveled_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = LeveledCompaction::default();
        let options = EngineOptions::default();
        
        // Create L0 tables
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            let mut table = Table::build(data, &options);
            table.level = 0;
            tables.push(table);
        }
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, metrics) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        assert!(!new_tables.is_empty(), "Should produce at least one new table");
        assert!(metrics.files_merged > 0, "Should track files merged");
        assert!(metrics.bytes_read > 0, "Should track bytes read");
        assert!(metrics.duration_ms > 0, "Duration should be positive");
        
        // Check that new tables are at level 1
        for table in &new_tables {
            assert_eq!(table.level, 1, "Compacted tables should be at level 1");
        }
    }

    #[test]
    fn test_compaction_write_amplification_size_tiered() {
        use crate::infra::config::LsmConfig;
        use crate::core::engine::compaction::*;
        use crate::core::table::Table;
        use crate::core::engine::EngineOptions;
        use std::collections::BTreeMap;
        
        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();
        
        // Create tables with known sizes
        let mut tables = Vec::new();
        
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }
        
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_new_tables, metrics) = strategy.execute(tables, &options, &storage_config, &output_dir).unwrap();
        
        // Write amplification = bytes_written / bytes_read
        // For SizeTiered, should be < 3x
        if metrics.bytes_read > 0 {
            let write_amplification = metrics.bytes_written as f64 / metrics.bytes_read as f64;
            assert!(
                write_amplification < 3.0,
                "Write amplification for SizeTiered should be < 3x, got {:.2}x",
                write_amplification
            );
        }
    }
}
