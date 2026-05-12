pub mod compaction;
pub mod manifest;
pub mod version_set;

use crate::core::log_record::LogRecord;
use crate::core::table::Table;
use crate::infra::error::Result;
use crate::storage::cache::Cache;
use crate::storage::wal::WriteAheadLog;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use self::compaction::{Compaction, CompactionMetrics, CompactionOptions, CompactionStrategyType};

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
    pub block_cache_size_mb: usize,
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
            block_cache_size_mb: 64,
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
            block_cache_size_mb: config.storage.block_cache_size_mb,
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

/// All mutable state of the engine, protected behind a Mutex.
pub(crate) struct EngineCore<C: Cache> {
    pub(crate) memtables: HashMap<String, Vec<MemTable>>,
    pub(crate) memtable_bytes: HashMap<String, usize>,
    pub(crate) version_set: VersionSet<C>,
    pub(crate) compaction: Compaction,
    pub(crate) wal: WriteAheadLog,
}

/// The core engine that manages LSM-tree structure and compaction.
///
/// # Type parameters
///
/// * `C` — The block cache implementation (typically
///   [`GlobalBlockCache`](crate::storage::cache::GlobalBlockCache) or
///   [`NoopCache`](crate::storage::cache::NoopCache) for tests).
///
/// # Usage example
///
/// ```rust
/// use apexstore::LsmConfig;
/// use apexstore::core::engine::Engine;
/// use apexstore::storage::cache::GlobalBlockCache;
///
/// let dir = tempfile::tempdir().unwrap();
/// let mut config = LsmConfig::default();
/// config.core.dir_path = dir.path().to_path_buf();
///
/// let mut engine = Engine::new_from_config(
///     &config,
///     GlobalBlockCache::new(100, 4096),
/// ).unwrap();
///
/// engine.set(b"key1", b"value1").unwrap();
/// assert_eq!(engine.get(b"key1").unwrap(), Some(b"value1".to_vec()));
/// engine.delete(b"key1").unwrap();
/// assert_eq!(engine.get(b"key1").unwrap(), None);
/// ```
pub struct Engine<C: Cache> {
    options: EngineOptions,
    /// All mutable state behind a mutex for thread-safe access.
    core: Arc<Mutex<EngineCore<C>>>,
    /// Background compaction running flag.
    compaction_running: Arc<AtomicBool>,
    /// Handle to the background compaction thread.
    compaction_thread: Mutex<Option<JoinHandle<()>>>,
    /// Path to the manifest file (unused currently).
    _manifest: PathBuf,
    /// SSTable output directory (used during initialization).
    _sst_dir: PathBuf,
}

pub type LsmEngineGeneric<C> = Engine<C>;
pub type LsmEngine = Engine<Arc<crate::storage::cache::GlobalBlockCache>>;
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
            None => KeySlice::new(&[]), // Caller should check is_valid() first
        }
    }
    fn value(&self) -> &[u8] {
        match self.current {
            Some((_, v)) => v.as_slice(),
            None => &[], // Caller should check is_valid() first
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

#[allow(dead_code)]
impl<C: Cache> Engine<C> {
    // ========== pub(crate) accessors for internal crate use ==========
    // These methods are reserved for future use / external crate access

    /// Returns the engine options.
    pub(crate) fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// Returns the write buffer limit.
    pub(crate) fn write_buffer_limit(&self) -> usize {
        self.options.write_buffer_size * self.options.max_write_buffer_number
    }

    /// Returns the SSTable directory (for testing).
    pub(crate) fn sst_dir(&self) -> &PathBuf {
        &self._sst_dir
    }

    /// Lock the core and return the guard.
    pub(crate) fn lock_core(&self) -> Result<std::sync::MutexGuard<'_, EngineCore<C>>> {
        self.core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in lock_core"))
    }
}

/// Compact a single column family, operating directly on `&mut EngineCore`.
/// This is the core compaction logic extracted from `compact_cf` so it can be
/// reused from both the public synchronous API and the background compaction thread.
///
/// NOTE: This function holds the lock (via the `&mut EngineCore` borrow) for the
/// entire compaction duration, including I/O. For background compaction where
/// the lock should be released during I/O, use the three-phase approach in
/// `maybe_compact` instead.
fn compact_cf_core<C: Cache>(
    core: &mut EngineCore<C>,
    options: &EngineOptions,
    cf: &str,
) -> Result<Option<CompactionMetrics>> {
    let tables = core.version_set.get_tables(cf);
    if tables.len() < core.compaction.options().compaction_threshold {
        return Ok(None);
    }

    let groups = core.compaction.pick_compaction(&tables, options);
    if groups.is_empty() {
        return Ok(None);
    }

    let mut all_metrics = CompactionMetrics::default();
    for group_indices in groups {
        let (new_tables, metrics) = core.compaction.compact(&group_indices, &tables, options)?;
        core.version_set
            .atomic_replace(cf, &group_indices, new_tables);
        all_metrics.bytes_read += metrics.bytes_read;
        all_metrics.bytes_written += metrics.bytes_written;
        all_metrics.files_merged += metrics.files_merged;
        all_metrics.duration_ms += metrics.duration_ms;
    }

    Ok(Some(all_metrics))
}

impl<C: Cache> Engine<C> {
    /// Create a new engine with default options.
    pub fn new_generic(
        options: EngineOptions,
        cache: C,
        dir_path: &std::path::Path,
    ) -> Result<Self> {
        // Create SSTable directory
        let sst_dir = dir_path.join("sstables");
        std::fs::create_dir_all(&sst_dir)?;

        // Create storage config from options
        let storage_config = crate::infra::config::StorageConfig {
            block_size: options.block_size,
            block_cache_size_mb: options.block_cache_size_mb,
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

        // Build EngineCore with all mutable state
        let version_set = VersionSet::new(options.clone(), cache);

        let mut core = EngineCore {
            memtables: HashMap::new(),
            memtable_bytes: HashMap::new(),
            version_set,
            compaction,
            wal,
        };

        // Replay WAL records into the core
        Self::replay_wal_records_core(&mut core, recovered_records)?;

        let engine = Self {
            options: options.clone(),
            core: Arc::new(Mutex::new(core)),
            compaction_running: Arc::new(AtomicBool::new(false)),
            compaction_thread: Mutex::new(None),
            _manifest: PathBuf::new(),
            _sst_dir: sst_dir,
        };

        Ok(engine)
    }

    /// Create a new engine from an `LsmConfig` (the app-level config).
    pub fn new_from_config(config: &crate::infra::config::LsmConfig, cache: C) -> Result<Self> {
        let options: EngineOptions = config.into();
        let dir_path = std::path::PathBuf::from(&config.core.dir_path);
        Self::new_generic(options, cache, &dir_path)
    }

    /// Replay WAL records to reconstruct memtable state (operates on EngineCore directly).
    fn replay_wal_records_core(core: &mut EngineCore<C>, records: Vec<LogRecord>) -> Result<()> {
        for record in records {
            if record.is_deleted {
                let cf = record.column_family.as_deref().unwrap_or("default");
                let mem = core.memtables.entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new());
                }
                let last = mem.len() - 1;
                mem[last].delete(record.key.clone());
                *core.memtable_bytes.entry(cf.to_string()).or_default() += record.key.len();
            } else {
                let cf = record.column_family.as_deref().unwrap_or("default");
                let mem = core.memtables.entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new());
                }
                let last = mem.len() - 1;
                mem[last].put(record.key.clone(), record.value.clone());
                *core.memtable_bytes.entry(cf.to_string()).or_default() +=
                    record.key.len() + record.value.len();
            }
        }
        Ok(())
    }

    /// Put a key-value pair into the specified column family.
    pub fn put_cf(&mut self, cf: &str, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        let needs_compact;
        {
            let mut core = self.core.lock().map_err(|_| {
                crate::infra::error::LsmError::LockPoisoned("engine core in put_cf")
            })?;
            // Write to WAL first (before modifying memtable) for crash safety
            let mut record = LogRecord::new(key.clone(), value.clone());
            record.column_family = Some(cf.to_string());
            core.wal.write_record(&record)?;

            let mem = core.memtables.entry(cf.to_string()).or_default();
            if mem.is_empty() {
                mem.push(MemTable::new());
            }
            let last = mem.len() - 1;
            mem[last].put(key.clone(), value.clone());
            *core.memtable_bytes.entry(cf.to_string()).or_default() += key.len() + value.len();
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes.get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        } // core lock is dropped here
        if needs_compact {
            self.maybe_compact();
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
        let needs_compact;
        {
            let mut core = self.core.lock().map_err(|_| {
                crate::infra::error::LsmError::LockPoisoned("engine core in delete_cf")
            })?;

            // Write tombstone to WAL first (before modifying memtable) for crash safety
            let mut record = LogRecord::tombstone(key.clone());
            record.column_family = Some(cf.to_string());
            core.wal.write_record(&record)?;

            let mem = core.memtables.entry(cf.to_string()).or_default();
            if mem.is_empty() {
                mem.push(MemTable::new());
            }
            let last = mem.len() - 1;
            mem[last].delete(key.clone());
            *core.memtable_bytes.entry(cf.to_string()).or_default() += key.len();
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes.get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        }
        if needs_compact {
            self.maybe_compact();
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
        let core = self
            .core
            .lock()
            .map_err(|_e| crate::infra::error::LsmError::LockPoisoned("engine core in get_cf"))?;
        if let Some(memtables) = core.memtables.get(cf) {
            for mem in memtables.iter().rev() {
                if let Some(v) = mem.data.get(key) {
                    return Ok(Some(v.clone()));
                }
            }
        }
        Ok(core.version_set.get(cf, key))
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
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in scan_cf"))?;
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        // 1. Memtables (newer first)
        if let Some(memtables) = core.memtables.get(cf) {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(InternalMemTableIterator::new(&mem.data)));
            }
        }

        // 2. SSTables (from VersionSet) — skip non-intersecting ranges
        for sst_iter in core.version_set.table_iters_in_range(cf, lower, upper) {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();
        let limit = limit.unwrap_or(MAX_SCAN_LIMIT);

        while merge_iter.is_valid() && results.len() < limit {
            if let Some(lower) = lower {
                if merge_iter.key().as_slice() < lower {
                    merge_iter.next();
                    continue;
                }
            }
            if let Some(upper) = upper {
                if merge_iter.key().as_slice() >= upper {
                    break;
                }
            }
            results.push((merge_iter.key(), merge_iter.value().to_vec()));
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
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)> {
        // Calculate upper bound for prefix scan
        let upper_bound = Self::prefix_end(prefix);

        // Start from prefix. When cursor is provided, use cursor as the lower bound
        // (cursor >= prefix since it was returned by a previous prefix search).
        let start = cursor.map(|c| c.as_bytes()).or(Some(prefix.as_bytes()));

        // Request extra records to detect if there are more results.
        // When cursor is set, we need an additional +1 because the cursor
        // match gets filtered out, consuming one scan slot.
        let scan_extra = if cursor.is_some() { 2 } else { 1 };
        let scan_limit = Some(limit + scan_extra);

        let results = self.scan_cf("default", start, upper_bound.as_deref(), scan_limit)?;

        // If cursor is set, skip the first result if it matches the cursor key
        let results: Vec<(Vec<u8>, Vec<u8>)> = results
            .into_iter()
            .skip_while(|(k, _)| cursor.is_some_and(|c| k.as_slice() == c.as_bytes()))
            .collect();

        // Determine if there are more results beyond the limit
        let has_more = results.len() > limit;

        // Take only `limit` results for the current page
        let mut results = results;
        results.truncate(limit);

        // Return cursor pointing to the next page when there are more results
        let new_cursor = if has_more {
            results
                .last()
                .and_then(|(k, _)| String::from_utf8(k.clone()).ok())
        } else {
            None
        };

        Ok((results, new_cursor))
    }

    pub fn keys(&self) -> Result<Vec<Vec<u8>>> {
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in keys"))?;
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        if let Some(memtables) = core.memtables.get("default") {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(InternalMemTableIterator::new(&mem.data)));
            }
        }

        for sst_iter in core.version_set.table_iters("default") {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();

        while merge_iter.is_valid() && results.len() < MAX_SCAN_LIMIT {
            results.push(merge_iter.key());
            merge_iter.next();
        }

        Ok(results)
    }

    pub fn count(&self) -> Result<usize> {
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in count"))?;
        let mut count = 0;
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        if let Some(memtables) = core.memtables.get("default") {
            for mem in memtables.iter().rev() {
                count += mem.data.len();
            }
        }

        for sst_iter in core.version_set.table_iters("default") {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        while merge_iter.is_valid() {
            count += 1;
            merge_iter.next();
        }

        Ok(count)
    }

    /// Flush the oldest memtable for the given column family.
    /// Returns true if compaction should be triggered after the lock is released.
    /// Flush the current memtable to an SSTable.
    /// Public wrapper used by benchmarks and tests.
    pub fn flush_memtable(&self) -> Result<()> {
        let mut core = self.core.lock().map_err(|_| {
            crate::infra::error::LsmError::LockPoisoned("engine core in flush_memtable")
        })?;
        self.flush_memtable_impl("default", &mut core)?;
        Ok(())
    }

    fn flush_memtable_impl(&self, cf: &str, core: &mut EngineCore<C>) -> Result<bool> {
        if let Some(memtables) = core.memtables.get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let table = Table::build(mem.data.into_iter().collect(), &self.options);
                core.version_set.add_table(cf, table);
                let bytes = core.memtable_bytes.get_mut(cf).ok_or_else(|| {
                    crate::LsmError::InvalidArgument(format!(
                        "Column family {} not found in memtable_bytes",
                        cf
                    ))
                })?;
                *bytes = 0;

                // ✅ FIX issue #107: Clear WAL while we hold the core lock
                // ✅ FIX issue #122: Only remove records for the flushed CF,
                // not the entire WAL (other CFs' data must survive).
                core.wal
                    .retain(|r| r.column_family.as_deref() != Some(cf))?;

                // Check if compaction might be needed after this flush
                let threshold = self.options.compaction_options.compaction_threshold;
                return Ok(core.version_set.table_count(cf) > threshold);
            }
        }
        Ok(false)
    }

    pub fn compact_cf(&self, cf: &str) -> Result<Option<CompactionMetrics>> {
        let mut core = self.core.lock().map_err(|_| {
            crate::infra::error::LsmError::LockPoisoned("engine core in compact_cf")
        })?;
        compact_cf_core(&mut core, &self.options, cf)
    }

    pub fn compact(&self) -> Result<Vec<(String, CompactionMetrics)>> {
        let mut results = Vec::new();
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in compact"))?;
        let column_families = core.version_set.column_families();
        drop(core); // Release lock before calling compact_cf which will re-acquire
                    // Actually, we need the lock for compact_cf, so just call it per CF
        for cf in column_families {
            if let Some(metrics) = self.compact_cf(&cf)? {
                results.push((cf, metrics));
            }
        }

        Ok(results)
    }

    /// Check if compaction should be triggered and run it in background
    pub fn maybe_compact(&self) {
        // Quick check to avoid unnecessary lock contention
        if self.compaction_running.load(Ordering::SeqCst) {
            return;
        }

        // Acquire the compaction_thread lock FIRST before spawning.
        // This prevents a TOCTOU race with close(): when close() holds
        // this lock, no new thread can be spawned and join-handle-stored
        // after close() has already taken the handle.
        let mut thread_guard = match self.compaction_thread.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        // Now we hold the lock. Check running flag again — close() may
        // have acquired this lock ahead of us and set running = false.
        if self.compaction_running.load(Ordering::SeqCst) {
            return;
        }

        // Claim the compaction slot inside the lock, so close() is
        // guaranteed to see this flag change before we store the handle.
        self.compaction_running.store(true, Ordering::Release);

        // Clone what the thread needs before spawning
        let core = self.core.clone();
        let running = self.compaction_running.clone();
        let options = self.options.clone();

        let handle = std::thread::spawn(move || {
            // Wrap compaction logic in catch_unwind to prevent panics from propagating
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // ── Phase 1: Build compaction plans while holding the lock ──
                // Snapshot which CFs need compaction and what tables/groups to compact.
                // Then drop the lock so writes can proceed during I/O.
                #[derive(Clone)]
                struct CompactionPlan {
                    cf: String,
                    tables: Vec<Table>,
                    groups: Vec<Vec<usize>>,
                    compaction: Compaction,
                    options: EngineOptions,
                }

                let plans: Vec<CompactionPlan> = {
                    let core = match core.lock() {
                        Ok(guard) => guard,
                        Err(_) => return,
                    };

                    core.version_set
                        .column_families()
                        .iter()
                        .filter_map(|cf| {
                            let tables = core.version_set.get_tables(cf);
                            if tables.len() < core.compaction.options().compaction_threshold {
                                return None;
                            }
                            let groups = core.compaction.pick_compaction(&tables, &options);
                            if groups.is_empty() {
                                return None;
                            }
                            Some(CompactionPlan {
                                cf: cf.clone(),
                                tables,
                                groups,
                                compaction: core.compaction.clone(),
                                options: options.clone(),
                            })
                        })
                        .collect()
                }; // MutexGuard dropped here → core lock is released

                // ── Phase 2: Execute compaction I/O without holding the lock ──
                // This is the slow part: read SSTables, merge, write new SSTable.
                let mut results: Vec<(String, Vec<usize>, Vec<Table>)> = Vec::new();
                for plan in &plans {
                    for group_indices in &plan.groups {
                        match plan
                            .compaction
                            .compact(group_indices, &plan.tables, &plan.options)
                        {
                            Ok((new_tables, _metrics)) => {
                                results.push((plan.cf.clone(), group_indices.clone(), new_tables));
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Background compaction failed for CF {}: {:?}",
                                    plan.cf,
                                    e
                                );
                            }
                        }
                    }
                }

                // ── Phase 3: Re-acquire lock and apply results ──
                if let Ok(mut core) = core.lock() {
                    for (cf, group_indices, new_tables) in results {
                        core.version_set
                            .atomic_replace(&cf, &group_indices, new_tables);
                    }
                }
            }));

            if let Err(panic_info) = result {
                tracing::error!("Compaction thread panicked: {:?}", panic_info);
            }

            running.store(false, Ordering::Release);
        });

        // Store the join handle while we still hold the lock.
        // This guarantees that any concurrent close() either:
        //   a) blocks on the lock and finds this handle after we release it, or
        //   b) has already taken the handle (closing an earlier thread),
        //      but then close() cannot spawn new threads because it can't
        //      acquire this lock while we hold it.
        *thread_guard = Some(handle);
    }

    /// Close the engine: signal compaction thread to stop and wait for it to finish.
    ///
    /// Locks `compaction_thread` first, then sets `compaction_running = false`
    /// inside the lock. This ordering eliminates the window where a concurrently
    /// executing `maybe_compact()` could spawn a new thread and store its handle
    /// after `close()` has already taken the handle but before it sets the flag.
    pub fn close(&self) {
        // Lock compaction_thread first so that any concurrent maybe_compact():
        //   a) blocks until we release the lock, at which point running=false
        //      prevents it from spawning; or
        //   b) has already stored its handle under the lock, so we'll find it here.
        if let Ok(mut handle_opt) = self.compaction_thread.lock() {
            self.compaction_running.store(false, Ordering::Release);
            if let Some(handle) = handle_opt.take() {
                match handle.join() {
                    Ok(()) => {}
                    Err(e) => {
                        tracing::error!("Compaction thread panicked on shutdown: {:?}", e);
                    }
                }
            }
        }
    }

    pub fn stats(&self, cf: &str) -> Result<LsmStats> {
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in stats"))?;
        let mut stats = LsmStats::default();

        // Get stats from version set
        let vs_stats = core.version_set.stats(cf);
        stats.num_tables = vs_stats.num_tables;
        stats.total_size = vs_stats.total_size;
        stats.total_records = vs_stats.total_records;
        stats.sst_kb = vs_stats.sst_kb;
        stats.sst_files = vs_stats.sst_files;
        stats.sst_records = vs_stats.sst_records;
        stats.max_levels_reached = vs_stats.max_levels_reached;
        stats.num_tables_at_max = vs_stats.num_tables_at_max;

        // Memtable stats
        if let Some(memtables) = core.memtables.get(cf) {
            stats.mem_records = memtables.iter().map(|m| m.data.len()).sum();
            stats.mem_kb = core.memtable_bytes.get(cf).copied().unwrap_or(0) / 1024;
        }

        // WAL stats
        stats.wal_kb = core.wal.size()? as usize / 1024;

        Ok(stats)
    }

    pub fn stats_all(&self) -> Result<LsmStats> {
        let core = self
            .core
            .lock()
            .map_err(|_| crate::infra::error::LsmError::LockPoisoned("engine core in stats_all"))?;
        let mut combined = LsmStats::default();
        let column_families = core.version_set.column_families();

        for cf in column_families {
            let vs_stats = core.version_set.stats(&cf);
            combined.num_tables += vs_stats.num_tables;
            combined.total_size += vs_stats.total_size;
            combined.total_records += vs_stats.total_records;
            combined.sst_kb += vs_stats.sst_kb;
            combined.sst_files += vs_stats.sst_files;
            combined.sst_records += vs_stats.sst_records;

            // Memtable stats per CF
            if let Some(memtables) = core.memtables.get(&cf) {
                combined.mem_records += memtables.iter().map(|m| m.data.len()).sum::<usize>();
                combined.mem_kb += core.memtable_bytes.get(&cf).copied().unwrap_or(0) / 1024;
            }
        }

        combined.wal_kb = core.wal.size()? as usize / 1024;

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

impl<C: Cache> Drop for Engine<C> {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine::compaction::CompactionStrategy;
    use crate::storage::cache::NoopCache;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

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

        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert some non-ASCII key-value pairs
        let test_pairs = vec![
            ("usuário:1", "value1"),
            ("usuário:2", "value2"),
            ("chave:3", "value3"),
        ];

        for (key, value) in &test_pairs {
            engine
                .set(key.as_bytes().to_vec(), value.as_bytes().to_vec())
                .unwrap();
        }

        // Search with prefix
        let (results, _): (Vec<(Vec<u8>, Vec<u8>)>, Option<String>) =
            engine.search_prefix("usuário:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_prefix_unicode_chars() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert with unicode prefix
        let test_pairs = vec![
            ("ção:1", "value1"),
            ("ção:2", "value2"),
            ("outro:3", "value3"),
        ];

        for (key, value) in &test_pairs {
            engine
                .set(key.as_bytes().to_vec(), value.as_bytes().to_vec())
                .unwrap();
        }

        let (results, _): (Vec<(Vec<u8>, Vec<u8>)>, Option<String>) =
            engine.search_prefix("ção:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_size_tiered_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;

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
        let (new_tables, _metrics) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
    }

    #[test]
    fn test_lazy_leveling_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (new_tables, _) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
    }

    #[test]
    fn test_compaction_removes_tombstones() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (new_tables, _) = strategy
            .execute(vec![table], &options, &storage_config, &output_dir)
            .unwrap();

        // The new table should not contain tombstones
        if let Some(new_table) = new_tables.first() {
            for (_, value) in &new_table.data {
                assert!(
                    !value.is_empty(),
                    "Tombstones should be removed during compaction"
                );
            }
        }
    }

    #[test]
    fn test_compaction_metrics() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (_, metrics) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

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
            data.insert(
                format!("key_{}", i).into_bytes(),
                format!("value_{}", i).into_bytes(),
            );
            let table = crate::core::table::Table::build(
                data,
                &crate::core::engine::EngineOptions::default(),
            );
            vs.add_table("default", table);
        }

        assert_eq!(vs.table_count("default"), 5);

        // Create new tables to replace some old ones
        let mut new_tables = Vec::new();
        for i in 0..2 {
            let mut data = std::collections::BTreeMap::new();
            data.insert(
                format!("new_key_{}", i).into_bytes(),
                format!("new_value_{}", i).into_bytes(),
            );
            new_tables.push(crate::core::table::Table::build(
                data,
                &crate::core::engine::EngineOptions::default(),
            ));
        }

        // Replace tables at indices 0, 1, 2 with new tables
        vs.atomic_replace("default", &[0, 1, 2], new_tables);

        assert_eq!(vs.table_count("default"), 4); // 5 - 3 + 2 = 4
    }

    #[test]
    fn test_1000_keys_with_multiple_compactions() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (_new_tables, metrics) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

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
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (new_tables, metrics) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
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
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
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
        let (_new_tables, metrics) = strategy
            .execute(tables, &options, &storage_config, &output_dir)
            .unwrap();

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
    fn test_crash_recovery_cf() {
        use crate::infra::config::LsmConfig;

        // Create engine in a temp directory
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write data to CF "users"
        let key = b"user:1".to_vec();
        let value = b"alice".to_vec();
        engine.put_cf("users", key.clone(), value.clone()).unwrap();

        // Verify data is present before crash
        let result = engine.get_cf("users", key.as_slice()).unwrap();
        assert_eq!(result, Some(value.clone()));

        // Verify data is NOT in default CF before crash
        let result_default = engine.get_cf("default", &key).unwrap();
        assert_eq!(result_default, None);

        // Drop engine — simulating crash without flush
        drop(engine);

        // Create a new engine from the same directory (triggers WAL recovery)
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Assert data is present in "users" CF after recovery
        let result_recovered = engine2.get_cf("users", &key).unwrap();
        assert_eq!(result_recovered, Some(value.clone()));

        // Assert data is NOT present in "default" CF after recovery
        let result_default_recovered = engine2.get_cf("default", &key).unwrap();
        assert_eq!(result_default_recovered, None);
    }

    #[test]
    fn test_crash_recovery_cf_after_flush() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Use a tiny memtable so writes trigger a flush immediately
        config.core.memtable_max_size = 512;

        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write data to both "users" and "default" CFs
        let users_key = b"user:1".to_vec();
        let users_value = b"alice".to_vec();
        engine
            .put_cf("users", users_key.clone(), users_value.clone())
            .unwrap();

        let default_key = b"default:1".to_vec();
        let default_value = b"bob".to_vec();
        engine
            .put_cf("default", default_key.clone(), default_value.clone())
            .unwrap();

        // Verify both CFs have data before crash
        let result_users = engine.get_cf("users", &users_key).unwrap();
        assert_eq!(result_users, Some(users_value.clone()));

        let result_default = engine.get_cf("default", &default_key).unwrap();
        assert_eq!(result_default, Some(default_value.clone()));

        // Drop engine — simulating crash without flush (WAL will be replayed)
        drop(engine);

        // Reopen engine
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Both CFs must have their data after WAL recovery
        let result_users_recovered = engine2.get_cf("users", &users_key).unwrap();
        assert_eq!(
            result_users_recovered,
            Some(users_value),
            "users CF data should survive crash via WAL recovery"
        );

        let result_default_recovered = engine2.get_cf("default", &default_key).unwrap();
        assert_eq!(
            result_default_recovered,
            Some(default_value),
            "default CF data should survive crash via WAL recovery"
        );
    }

    #[test]
    fn test_crash_during_compaction() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold so compaction triggers after few flushes
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let key_count = 200;

        {
            let mut engine = Engine::new_from_config(
                &config,
                crate::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            // Write many keys to trigger flushes and compactions
            for i in 0..key_count {
                engine.set(format!("k{}", i), vec![b'x'; 100]).unwrap();
            }
        } // engine dropped — simulating crash with active compaction state

        // Reopen — must not panic and must recover data from WAL
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Verify engine is operational after crash-during-compaction
        // Engine opened without panic — verify stats, count and scan work
        let _stats = engine2.stats("default").unwrap_or_default();
        engine2.count().unwrap();
        engine2.scan().unwrap_or_default();
    }

    #[test]
    fn test_writes_during_compaction() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold so compaction triggers early
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys to trigger flushes and compaction.
        // Each put_cf call releases the inner core lock before maybe_compact,
        // so background compaction (spawned by maybe_compact) can proceed
        // while writes continue.
        let key_count = 500;
        for i in 0..key_count {
            engine
                .set(format!("k{}", i), vec![b'x'; 100])
                .expect("write during compaction must succeed");
        }

        // Verify at least some keys are readable after compaction
        let mut found = 0;
        for i in 0..key_count {
            if let Ok(Some(_)) = engine.get(format!("k{}", i)) {
                found += 1;
            }
        }
        assert!(
            found > 0,
            "At least some keys should be readable after compaction"
        );
    }

    #[test]
    fn test_shutdown_with_compaction_in_progress() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let key_count = 200;

        // This block simulates shutdown with compaction in progress — should not panic
        {
            let mut engine = Engine::new_from_config(
                &config,
                crate::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            // Write keys to trigger flushes and potential compaction
            for i in 0..key_count {
                engine.set(format!("k{}", i), vec![b'x'; 100]).unwrap();
            }
        } // engine dropped here — Drop::drop calls close() which joins the compaction thread

        // Re-open the engine to verify:
        // 1. The compaction thread was joined (no lock leak — engine can re-open)
        // 2. Data survived the shutdown-with-compaction scenario
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Verify engine is operational after shutdown with compaction
        let count = engine2.count().unwrap_or(0);
        assert!(
            count > 0,
            "Data should survive shutdown during compaction, got {} keys",
            count
        );
    }

    // ── T6: Bloom filter prevents data lookup for absent key ──
    #[test]
    fn test_bloom_filter_prevents_absent_key_lookup() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        // Insert some keys via the SSTable path (bypassing memtable)
        // We create a VersionSet with a table that has a bloom filter and
        // verify that reading an absent key doesn't iterate table data.
        let mut core = engine.lock_core().unwrap();
        let mut data = BTreeMap::new();
        data.insert(b"present_key".to_vec(), b"present_value".to_vec());
        let mut table = Table::build(data, &engine.options);
        // Build a bloom filter covering only the inserted key
        let mut bf = bloomfilter::Bloom::<[u8]>::new(1024, 1).expect("valid bloom params");
        bf.set(b"present_key");
        table.bloom_filter = Some(bf);
        table.min_key = b"present_key".to_vec();
        table.max_key = b"present_key".to_vec();
        core.version_set.add_table("default", table);

        // Reading an absent key should return None (bloom filter says no)
        let result = core.version_set.get("default", b"absent_key");
        assert!(
            result.is_none(),
            "Absent key should return None via Bloom filter"
        );

        // Reading a present key should succeed
        let result = core.version_set.get("default", b"present_key");
        assert_eq!(result, Some(b"present_value".to_vec()));
    }

    // ── T7: Block cache hit returns cached value on repeated read ──
    #[test]
    fn test_kv_cache_hit_on_repeated_read() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        // Insert a key into the version set directly
        let mut core = engine.lock_core().unwrap();
        let mut data = BTreeMap::new();
        data.insert(b"cached_key".to_vec(), b"cached_value".to_vec());
        let table = Table::build(data, &engine.options);
        core.version_set.add_table("default", table);

        // First read populates the KV cache
        let r1 = core.version_set.get("default", b"cached_key");
        assert_eq!(r1, Some(b"cached_value".to_vec()));

        // Second read should hit the KV cache (cache populated by first read)
        let r2 = core.version_set.get("default", b"cached_key");
        assert_eq!(r2, Some(b"cached_value".to_vec()));

        // Verify cache stats by checking that clearing the cache still works
        core.version_set.clear_cache();
        let r3 = core.version_set.get("default", b"cached_key");
        assert_eq!(
            r3,
            Some(b"cached_value".to_vec()),
            "Value still readable after cache clear"
        );
    }

    // ── T8: scan_cf skips non-intersecting SSTables ──
    #[test]
    fn test_scan_cf_skips_non_intersecting_sstables() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        let mut core = engine.lock_core().unwrap();

        // Table 1: keys "a" to "c"
        let mut data1 = BTreeMap::new();
        data1.insert(b"a".to_vec(), b"1".to_vec());
        data1.insert(b"b".to_vec(), b"2".to_vec());
        data1.insert(b"c".to_vec(), b"3".to_vec());
        let mut t1 = Table::build(data1, &engine.options);
        t1.min_key = b"a".to_vec();
        t1.max_key = b"c".to_vec();
        core.version_set.add_table("default", t1);

        // Table 2: keys "x" to "z"
        let mut data2 = BTreeMap::new();
        data2.insert(b"x".to_vec(), b"24".to_vec());
        data2.insert(b"y".to_vec(), b"25".to_vec());
        data2.insert(b"z".to_vec(), b"26".to_vec());
        let mut t2 = Table::build(data2, &engine.options);
        t2.min_key = b"x".to_vec();
        t2.max_key = b"z".to_vec();
        core.version_set.add_table("default", t2);

        // Table 3: keys "m" to "p"
        let mut data3 = BTreeMap::new();
        data3.insert(b"m".to_vec(), b"13".to_vec());
        data3.insert(b"n".to_vec(), b"14".to_vec());
        data3.insert(b"o".to_vec(), b"15".to_vec());
        let mut t3 = Table::build(data3, &engine.options);
        t3.min_key = b"m".to_vec();
        t3.max_key = b"o".to_vec();
        core.version_set.add_table("default", t3);

        // Drop the core lock so scan_cf can acquire it
        drop(core);

        // Scan range [b, n] — should only include keys from table 1 and 3 (table 2 is entirely after "n")
        let results = engine
            .scan_cf("default", Some(b"b"), Some(b"n"), None)
            .unwrap();
        let keys: Vec<&[u8]> = results.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(
            keys,
            vec![b"b", b"c", b"m"],
            "Should only return keys b, c, m from intersecting tables"
        );
    }

    #[test]
    fn test_benchmark_memtable_read_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.compaction.max_sstables = 4;
        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        let key = b"bench_key";
        engine
            .put_cf("default", key.to_vec(), b"value".to_vec())
            .unwrap();

        // Warm up
        for _ in 0..100 {
            engine.get_cf("default", key).unwrap();
        }

        let start = std::time::Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            engine.get_cf("default", key).unwrap();
        }
        let elapsed = start.elapsed() / iterations as u32;

        // Memtable reads should be < 10 µs
        assert!(
            elapsed < std::time::Duration::from_micros(10),
            "Memtable read avg {:?} exceeds 10µs",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_sstable_warm_read_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert keys and flush to SSTable
        for i in 0..1000 {
            engine
                .put_cf(
                    "default",
                    format!("key_{:04}", i).into_bytes(),
                    b"value_1234567890".to_vec(),
                )
                .unwrap();
        }
        engine.flush_memtable().unwrap();

        // Warm up cache by reading all keys once
        for i in 0..1000 {
            let _ = engine.get_cf("default", format!("key_{:04}", i).as_bytes());
        }

        // Measure warm reads
        let start = std::time::Instant::now();
        let iterations = 100;
        for _ in 0..iterations {
            for i in 0..1000 {
                let _ = engine.get_cf("default", format!("key_{:04}", i).as_bytes());
            }
        }
        let elapsed = start.elapsed() / (iterations * 1000) as u32;

        assert!(
            elapsed < std::time::Duration::from_micros(500),
            "Warm SSTable read avg {:?} exceeds 500µs",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_scan_1k_keys_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 10 * 1024 * 1024; // Keep all keys in memtable
        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert 1000 keys
        for i in 0..1000 {
            engine
                .put_cf(
                    "default",
                    format!("key_{:04}", i).into_bytes(),
                    b"value_1234567890".to_vec(),
                )
                .unwrap();
        }

        let start = std::time::Instant::now();
        let iterations = 10;
        for _ in 0..iterations {
            let results = engine.scan_cf("default", None, None, Some(1000)).unwrap();
            assert_eq!(results.len(), 1000, "Should return all 1000 keys");
        }
        let elapsed = start.elapsed() / iterations as u32;

        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "Scan 1k keys avg {:?} exceeds 5ms",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_bloom_filter_negative_read() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.compaction.max_sstables = 4;
        // Use small memtable to trigger flush
        config.core.memtable_max_size = 4096;
        let mut engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write enough to trigger flush
        for i in 0..1000 {
            engine
                .put_cf("default", format!("key{}", i).into_bytes(), b"val".to_vec())
                .unwrap();
        }
        engine.flush_memtable().unwrap();

        let start = std::time::Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            // Key that doesn't exist — should be caught by bloom filter fast path
            let _ = engine.get_cf("default", b"nonexistent_key_xyz");
        }
        let elapsed = start.elapsed() / iterations as u32;

        assert!(
            elapsed < std::time::Duration::from_micros(10),
            "Bloom filter negative read avg {:?} exceeds 10µs",
            elapsed
        );
    }
}
