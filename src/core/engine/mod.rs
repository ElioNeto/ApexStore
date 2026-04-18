pub mod compaction;
pub mod manifest;
pub mod version_set;

use std::collections::HashMap;
use crate::core::iterators::StorageIterator;
use crate::core::key::KeySlice;
use crate::core::table::Table;
use crate::core::version::Version;
use crate::storage::cache::Cache;
use crate::storage::sst_iterator::SstIterator;

use self::compaction::Compaction;
use self::manifest::Manifest;
use self::version_set::VersionSet;

pub const DEFAULT_SCAN_LIMIT: usize = 128;
pub const MAX_SCAN_LIMIT: usize = 1024;

#[derive(Debug, Clone, Default)]
pub struct LsmStats {
    pub num_tables: usize,
    pub total_size: usize,
    pub total_records: usize,
    pub sst_kb: usize,
    pub wal_kb: usize,
    pub memtable_max_size: usize,
    pub mem_kb: usize,
    pub mem_records: usize,
    pub sst_files: usize,
    pub sst_records: usize,
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
    pub compaction_options: Compaction,
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
            compaction_options: Compaction::default(),
        }
    }
}

/// The core engine that manages LSM-tree structure and compaction.
pub struct Engine<C: Cache> {
    options: EngineOptions,
    manifest: Manifest,
    version_set: VersionSet<C>,
    /// Memtables indexed by column family.
    memtables: HashMap<String, Vec<MemTable>>,
    /// Write buffer limit in bytes per column family.
    write_buffer_limit: usize,
    /// Current total bytes in memtables per column family.
    memtable_bytes: HashMap<String, usize>,
}

pub type LsmEngineGeneric<C> = Engine<C>;
pub type LsmEngine = Engine<crate::storage::cache::GlobalBlockCache>;

struct MemTable {
    data: HashMap<Vec<u8>, Vec<u8>>,
    size: usize,
}

impl MemTable {
    fn new() -> Self {
        Self {
            data: HashMap::new(),
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

    fn iter(&self) -> impl Iterator<Item = (Vec<u8>, Vec<u8>)> + '_ {
        self.data.iter().map(|(k, v)| (k.clone(), v.clone()))
    }
}

impl<C: Cache> Engine<C> {
    pub fn new_generic(options: EngineOptions, cache: C) -> Self {
        Self {
            options: options.clone(),
            manifest: Manifest::new(),
            version_set: VersionSet::new(options.clone(), cache),
            memtables: HashMap::new(),
            write_buffer_limit: options.write_buffer_size * options.max_write_buffer_number,
            memtable_bytes: HashMap::new(),
        }
    }
}

impl Engine<crate::storage::cache::GlobalBlockCache> {
    pub fn new(config: crate::infra::config::LsmConfig) -> crate::infra::error::Result<Self> {
        let options = EngineOptions::default();
        let cache = crate::storage::cache::GlobalBlockCache::new(
            config.storage.block_cache_size_mb,
            config.storage.block_size,
        );
        Ok(Engine {
            options: options.clone(),
            manifest: Manifest::new(),
            version_set: VersionSet::new(options.clone(), (*cache).clone()),
            memtables: HashMap::new(),
            write_buffer_limit: options.write_buffer_size * options.max_write_buffer_number,
            memtable_bytes: HashMap::new(),
        })
    }
}

impl<C: Cache> Engine<C> {

    pub fn put_cf(&mut self, cf: &str, key: Vec<u8>, value: Vec<u8>) -> crate::infra::error::Result<()> {
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].put(key.clone(), value.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len() + value.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            self.flush_memtable(cf);
        }
        Ok(())
    }

    pub fn set<K, V>(&mut self, key: K, value: V) -> crate::infra::error::Result<()> 
    where K: Into<Vec<u8>>, V: Into<Vec<u8>> {
        self.put_cf("default", key.into(), value.into())
    }

    pub fn delete_cf<K>(&mut self, cf: &str, key: K) -> crate::infra::error::Result<()> 
    where K: Into<Vec<u8>> {
        let key = key.into();
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].delete(key.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            self.flush_memtable(cf);
        }
        Ok(())
    }

    pub fn delete<K>(&mut self, key: K) -> crate::infra::error::Result<()> 
    where K: Into<Vec<u8>> {
        self.delete_cf("default", key)
    }

    pub fn get_cf<K>(&self, cf: &str, key: K) -> crate::infra::error::Result<Option<Vec<u8>>> 
    where K: AsRef<[u8]> {
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

    pub fn get<K>(&self, key: K) -> crate::infra::error::Result<Option<Vec<u8>>> 
    where K: AsRef<[u8]> {
        self.get_cf("default", key)
    }



    pub fn scan(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        limit: Option<usize>,
    ) -> crate::infra::error::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut results = Vec::new();

        // Include memtables first (newest writes first).
        if let Some(memtables) = self.memtables.get(cf) {
            for mem in memtables.iter().rev() {
                for (k, v) in mem.iter() {
                    if let Some(lb) = lower {
                        if k.as_slice() < lb {
                            continue;
                        }
                    }
                    if let Some(ub) = upper {
                        if k.as_slice() >= ub {
                            continue;
                        }
                    }
                    results.push((k, v));
                    if let Some(l) = limit {
                        if results.len() >= l {
                            return Ok(results);
                        }
                    }
                }
            }
        }
        Ok(results)
    }

    pub fn keys(&self) -> crate::infra::error::Result<Vec<Vec<u8>>> {
        Ok(self.memtables.get("default").map_or(Vec::new(), |m| {
            m.iter().flat_map(|mt| mt.data.keys().cloned()).collect()
        }))
    }

    pub fn count(&self) -> crate::infra::error::Result<usize> {
        Ok(self.memtables.get("default").map_or(0, |m| {
            m.iter().map(|mt| mt.data.len()).sum()
        }))
    }

    pub fn stats(&self) -> crate::infra::error::Result<LsmStats> {
        Ok(LsmStats::default())
    }

    pub fn stats_all(&self) -> crate::infra::error::Result<Vec<(String, LsmStats)>> {
        Ok(vec![("default".to_string(), LsmStats::default())])
    }

    pub fn search(&self, _query: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        Vec::new()
    }

    pub fn search_prefix_legacy(&self, _prefix: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        Vec::new()
    }


    fn flush_memtable(&mut self, cf: &str) {
        if let Some(memtables) = self.memtables.get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let table = Table::build(mem.data.into_iter().collect(), &self.options);
                self.version_set.add_table(cf, table);
                *self.memtable_bytes.get_mut(cf).unwrap() = 0;
            }
        }
    }

    pub fn force_flush(&mut self) {
        let keys: Vec<String> = self.memtables.keys().cloned().collect();
        for cf in keys {
            self.flush_memtable(&cf);
        }
    }

    pub fn compact(&mut self) {
        // Level-based compaction with tiered compaction strategy.
        let version: crate::core::version::Version<C> = self.version_set.current_version();
        for level in 0..self.options.max_levels.saturating_sub(1) {
            let level_tables = version.get_level_tables(level);
            if level_tables.len() < 2 {
                continue;
            }

            // Check if compaction is needed: any table smaller than min_table_size_to_compact
            // or total size exceeding a threshold triggers compaction.
            let total_size: usize = level_tables.iter().map(|t| t.size()).sum();
            let needs_compaction = level_tables
                .iter()
                .any(|t| t.size() < self.options.min_table_size_to_compact)
                || total_size > self.options.max_table_size * 2;

            if !needs_compaction {
                continue;
            }

            // Pick tables to compact: for level 0, compact all; otherwise compact by size.
            let mut tables_to_compact = level_tables;
            if level == 0 {
                // Level 0: compact all overlapping tables.
            } else {
                // Higher levels: ensure we don't compact too many at once.
                tables_to_compact = tables_to_compact
                    .into_iter()
                    .take(self.options.compaction_options.max_tables_per_compaction)
                    .collect();
            }

            // Verify compaction reduces SSTable count (important invariant).
            let before_count = tables_to_compact.len();
            if before_count <= 1 {
                continue;
            }

            // Build merged table.
            let mut iterators: Vec<Box<dyn StorageIterator<KeyType = KeySlice>>> =
                tables_to_compact
                    .into_iter()
                    .map(|t| Box::new(t.iter()) as Box<dyn StorageIterator<KeyType = KeySlice>>)
                    .collect();

            let mut merged_data = HashMap::new();
            let mut current_key: Option<Vec<u8>> = None;
            let mut current_value: Option<Vec<u8>> = None;

            loop {
                let mut min_idx = None;
                let mut min_key: Option<KeySlice> = None;

                for (idx, iter) in iterators.iter_mut().enumerate() {
                    if iter.is_valid() {
                        let key = iter.key();
                        if min_key
                            .as_ref()
                            .map_or(true, |min| key.as_slice() < min.as_slice())
                        {
                            min_key = Some(key);
                            min_idx = Some(idx);
                        }
                    }
                }

                if let Some(idx) = min_idx {
                    let key = iterators[idx].key().to_vec();
                    let value = iterators[idx].value().to_vec();
                    iterators[idx].next();

                    // Resolve write conflicts: last write wins (insert order in iterators is by level+offset).
                    merged_data.insert(key, value);
                } else {
                    break;
                }
            }

            // Ensure compaction reduces SSTable count: remove old tables and add new one.
            if merged_data.len() > 0 {
                let new_table = Table::build(merged_data, &self.options);
                // Remove compacted tables and add new merged table.
                // In a real implementation, we'd track table metadata and generation numbers.
                // Here we simulate by rebuilding the level with the new table.
                self.version_set.remove_and_add_table(level, new_table);
            }
        }
    }
}
