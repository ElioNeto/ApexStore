pub mod compaction;
pub mod manifest;
pub mod version_set;

use crate::core::table::Table;
use crate::storage::cache::Cache;
use std::collections::HashMap;

use self::compaction::Compaction;
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
    _manifest: Manifest,
    version_set: VersionSet<C>,
    /// Memtables indexed by column family.
    memtables: HashMap<String, Vec<MemTable>>,
    /// Write buffer limit in bytes per column family.
    write_buffer_limit: usize,
    /// Current total bytes in memtables per column family.
    memtable_bytes: HashMap<String, usize>,
    /// Compaction policy.
    #[allow(dead_code)]
    compaction_options: Compaction,
    /// Compaction executor.
    #[allow(dead_code)]
    compaction: Compaction,
}

pub type LsmEngineGeneric<C> = Engine<C>;
pub type LsmEngine = Engine<crate::storage::cache::GlobalBlockCache>;
pub type ScanRangeResult = crate::infra::error::Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)>;

struct MemTable {
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
        KeySlice::new(self.current.unwrap().0.as_slice())
    }
    fn value(&self) -> &[u8] {
        self.current.unwrap().1.as_slice()
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
    pub fn new_generic(options: EngineOptions, cache: C) -> Self {
        Self {
            options: options.clone(),
            _manifest: Manifest::new(),
            version_set: VersionSet::new(options.clone(), cache),
            memtables: HashMap::new(),
            write_buffer_limit: options.write_buffer_size * options.max_write_buffer_number,
            memtable_bytes: HashMap::new(),
            compaction_options: options.compaction_options.clone(),
            compaction: Compaction::default(),
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
            _manifest: Manifest::new(),
            version_set: VersionSet::new(options.clone(), (*cache).clone()),
            memtables: HashMap::new(),
            write_buffer_limit: options.write_buffer_size * options.max_write_buffer_number,
            memtable_bytes: HashMap::new(),
            compaction_options: options.compaction_options.clone(),
            compaction: Compaction::default(),
        })
    }
}

impl<C: Cache> Engine<C> {
    pub fn put_cf(
        &mut self,
        cf: &str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> crate::infra::error::Result<()> {
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].put(key.clone(), value.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len() + value.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            self.flush_memtable_impl(cf);
        }
        Ok(())
    }

    pub fn set<K, V>(&mut self, key: K, value: V) -> crate::infra::error::Result<()>
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        self.put_cf("default", key.into(), value.into())
    }

    pub fn delete_cf<K>(&mut self, cf: &str, key: K) -> crate::infra::error::Result<()>
    where
        K: Into<Vec<u8>>,
    {
        let key = key.into();
        let mem = self.memtables.entry(cf.to_string()).or_default();
        if mem.is_empty() {
            mem.push(MemTable::new());
        }
        let last = mem.len() - 1;
        mem[last].delete(key.clone());
        *self.memtable_bytes.entry(cf.to_string()).or_default() += key.len();
        if self.memtable_bytes[cf] >= self.write_buffer_limit {
            self.flush_memtable_impl(cf);
        }
        Ok(())
    }

    pub fn delete<K>(&mut self, key: K) -> crate::infra::error::Result<()>
    where
        K: Into<Vec<u8>>,
    {
        self.delete_cf("default", key)
    }

    pub fn get_cf<K>(&self, cf: &str, key: K) -> crate::infra::error::Result<Option<Vec<u8>>>
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

    pub fn get<K>(&self, key: K) -> crate::infra::error::Result<Option<Vec<u8>>>
    where
        K: AsRef<[u8]>,
    {
        self.get_cf("default", key)
    }

    pub fn scan(&self) -> crate::infra::error::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan_cf("default", None, None, None)
    }

    pub fn scan_cf(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        limit: Option<usize>,
    ) -> crate::infra::error::Result<Vec<(Vec<u8>, Vec<u8>)>> {
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

        // Se houver search bound inferior, seek
        if let Some(lb) = lower {
            // Nota: Nosso MergeIterator.seek ainda não está implementado, mas para scans básicos
            // podemos apenas skipar. No futuro, seek otimizado seria preferível.
            while merge_iter.is_valid() && merge_iter.key().as_slice() < lb {
                merge_iter.next();
            }
        }

        let mut results = Vec::new();
        while merge_iter.is_valid() {
            let key = merge_iter.key();
            let key_slice: &[u8] = key.as_ref();

            // Check upper bound
            if let Some(ub) = upper {
                if key_slice >= ub {
                    break;
                }
            }

            // Apenas adicionar se não for tombstone (valor vazio em algumas impls, mas aqui vamos assumir todos válidos por enquanto)
            results.push((key_slice.to_vec(), merge_iter.value().to_vec()));

            if let Some(l) = limit {
                if results.len() >= l {
                    break;
                }
            }
            merge_iter.next();
        }

        Ok(results)
    }

    #[allow(clippy::type_complexity)]
    pub fn scan_range(
        &self,
        lower: Option<&str>,
        upper: Option<&str>,
        limit: usize,
    ) -> crate::infra::error::Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)> {
        let l_bytes = lower.map(|s| {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            b
        });
        let u_bytes = upper.map(|s| s.as_bytes());

        let results = self.scan_cf("default", l_bytes.as_deref(), u_bytes, Some(limit))?;
        let next_cursor = if results.len() >= limit && !results.is_empty() {
            Some(String::from_utf8_lossy(&results.last().unwrap().0).to_string())
        } else {
            None
        };
        Ok((results, next_cursor))
    }

    #[allow(clippy::type_complexity)]
    pub fn search_prefix(
        &self,
        prefix: &str,
        _cursor: Option<&str>,
        limit: usize,
    ) -> crate::infra::error::Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)> {
        // Simple implementation: scan with prefix as lower bound
        let results = self.scan_cf("default", Some(prefix.as_bytes()), None, Some(limit))?;
        // Filter results that actually start with prefix
        let filtered: Vec<_> = results
            .into_iter()
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .collect();
        Ok((filtered, None))
    }

    pub fn keys(&self) -> crate::infra::error::Result<Vec<Vec<u8>>> {
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
            results.push(merge_iter.key());
            merge_iter.next();
        }

        Ok(results)
    }

    pub fn count(&self) -> crate::infra::error::Result<usize> {
        let mem_count: usize = self
            .memtables
            .get("default")
            .map_or(0, |m| m.iter().map(|mt| mt.data.len()).sum());

        let sst_count = self.version_set.record_count("default");

        Ok(mem_count + sst_count)
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

    pub fn flush_memtable(&mut self) -> crate::infra::error::Result<()> {
        self.flush_memtable_impl("default");
        Ok(())
    }

    fn flush_memtable_impl(&mut self, cf: &str) {
        if let Some(memtables) = self.memtables.get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let table = Table::build(mem.data.into_iter().collect(), &self.options);
                self.version_set.add_table(cf, table);
                *self.memtable_bytes.get_mut(cf).unwrap() = 0;

                // ✅ FIX issue #105: acionar compactação se SSTable count exceder threshold
                let threshold = self.options.compaction_options.compaction_threshold;
                if self.version_set.table_count(cf) > threshold {
                    self.compact_cf(cf);
                }
            }
        }
    }

    pub fn compact_cf(&mut self, cf: &str) {
        let tables = self.version_set.drain_tables(cf);
        if let Some(merged) = Compaction::merge_tables(tables, &self.options) {
            self.version_set.remove_and_add_table(cf, merged);
        }
    }

    pub fn compact(&mut self) {
        // Compact all column families
        let column_families: Vec<String> = self.memtables.keys().cloned().collect();

        for cf in column_families {
            let tables = self.version_set.drain_tables(&cf);
            if !tables.is_empty() {
                if let Some(merged) = Compaction::merge_tables(tables, &self.options) {
                    self.version_set.remove_and_add_table(&cf, merged);
                }
            }
        }
    }
}
