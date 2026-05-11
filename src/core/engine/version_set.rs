use crate::storage::cache::Cache;
use crate::storage::cache::GlobalBlockCache;
use std::sync::Arc;

/// Statistics returned by `VersionSet::stats()`.
pub struct VersionStats {
    pub num_tables: usize,
    pub total_size: usize,
    pub total_records: usize,
    pub sst_kb: usize,
    pub sst_files: usize,
    pub sst_records: usize,
    pub max_levels_reached: usize,
    pub num_tables_at_max: usize,
}

pub struct VersionSet<C: Cache> {
    _cache: std::marker::PhantomData<C>,
    tables: std::collections::HashMap<String, Vec<crate::core::table::Table>>,
    block_cache: Option<Arc<GlobalBlockCache>>,
}

impl<C: Cache> VersionSet<C> {
    pub fn new(_options: crate::core::engine::EngineOptions, _cache: C) -> Self {
        Self {
            _cache: std::marker::PhantomData,
            tables: std::collections::HashMap::new(),
            block_cache: None,
        }
    }

    /// Set the block cache for this VersionSet (used for SstableReader passthrough)
    pub fn set_block_cache(&mut self, block_cache: Arc<GlobalBlockCache>) {
        self.block_cache = Some(block_cache);
    }

    pub fn get(&self, cf: &str, key: &[u8]) -> Option<Vec<u8>> {
        if let Some(cf_tables) = self.tables.get(cf) {
            'table_loop: for table in cf_tables.iter().rev() {
                // Skip tables whose key range doesn't include the target key
                if !table.min_key.is_empty() && !table.max_key.is_empty() {
                    if key < table.min_key.as_slice() || key > table.max_key.as_slice() {
                        continue;
                    }
                }

                // For persisted tables, check bloom filter before BTreeMap lookup
                if let Some(ref path) = table.path {
                    if let Some(ref block_cache) = self.block_cache {
                        if let Ok(reader) = crate::storage::reader::SstableReader::open(
                            path.clone(),
                            crate::infra::config::StorageConfig::default(),
                            block_cache.clone(),
                        ) {
                            if !reader.might_contain(key) {
                                // Bloom filter says key definitely does not exist -> skip
                                continue 'table_loop;
                            }
                            // Bloom says key might exist, fall through to BTreeMap lookup
                        }
                    }
                }

                if let Some(val) = table.data.get(key) {
                    return Some(val.clone());
                }
            }
        }
        None
    }

    pub fn scan(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        limit: Option<usize>,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut results = Vec::new();
        if let Some(cf_tables) = self.tables.get(cf) {
            for table in cf_tables.iter().rev() {
                for (k, v) in &table.data {
                    if lower.is_none_or(|lb| k.as_slice() >= lb)
                        && upper.is_none_or(|ub| k.as_slice() < ub)
                    {
                        results.push((k.clone(), v.clone()));
                    }
                    if let Some(l) = limit {
                        if results.len() >= l {
                            return results;
                        }
                    }
                }
            }
        }
        results
    }

    pub fn add_table(&mut self, cf: &str, table: crate::core::table::Table) {
        self.tables.entry(cf.to_string()).or_default().push(table);
    }

    pub fn current_version(&self) -> crate::core::version::Version<C> {
        crate::core::version::Version::new()
    }

    pub fn table_count(&self, cf: &str) -> usize {
        self.tables.get(cf).map_or(0, |v| v.len())
    }

    pub fn table_iters(&self, cf: &str) -> Vec<crate::core::table::TableIterator<'_>> {
        self.tables
            .get(cf)
            .map(|v| v.iter().rev().map(|t| t.iter()).collect())
            .unwrap_or_default()
    }

    /// Return table iterators whose [min_key, max_key] intersect the given range.
    /// This avoids creating unnecessary iterators for tables that cannot
    /// contain any key in the query range.
    pub fn table_iters_in_range(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
    ) -> Vec<crate::core::table::TableIterator<'_>> {
        self.tables
            .get(cf)
            .map(|v| {
                v.iter()
                    .rev()
                    .filter(|t| {
                        // Skip tables that have no range metadata (treat as always included)
                        if t.min_key.is_empty() || t.max_key.is_empty() {
                            return true;
                        }
                        // Table's max_key is before the query lower bound → no intersection
                        if let Some(lower) = lower {
                            if t.max_key.as_slice() < lower {
                                return false;
                            }
                        }
                        // Table's min_key is at or after the query upper bound → no intersection
                        if let Some(upper) = upper {
                            if t.min_key.as_slice() >= upper {
                                return false;
                            }
                        }
                        true
                    })
                    .map(|t| t.iter())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn record_count(&self, cf: &str) -> usize {
        self.tables
            .get(cf)
            .map_or(0, |v| v.iter().map(|t| t.data.len()).sum())
    }

    pub fn drain_tables(&mut self, cf: &str) -> Vec<crate::core::table::Table> {
        self.tables.remove(cf).unwrap_or_default()
    }

    pub fn remove_and_add_table(&mut self, cf: &str, new_table: crate::core::table::Table) {
        // Remove todas as tabelas da CF e substitui pela tabela compactada.
        let entry = self.tables.entry(cf.to_string()).or_default();
        entry.clear();
        entry.push(new_table);
    }

    /// Get all tables for a column family (without draining)
    pub fn get_tables(&self, cf: &str) -> Vec<crate::core::table::Table> {
        self.tables
            .get(cf)
            .map_or_else(Vec::new, |v| v.clone())
    }

    /// Atomically replace specific tables with new ones.
    ///
    /// New tables are inserted at the position of the first (minimum-index) removed table,
    /// preserving the invariant that tables in the Vec are ordered oldest-first.
    /// This prevents stale-data reads when flushes add tables during three-phase
    /// compaction's Phase 2 (I/O without core lock), because the compacted result
    /// is placed BEFORE the flushed tables in the Vec. Since `get()` iterates in
    /// reverse (`.rev()`), flushed tables (newer data) are checked first.
    pub fn atomic_replace(
        &mut self,
        cf: &str,
        indices: &[usize],
        new_tables: Vec<crate::core::table::Table>,
    ) {
        if let Some(tables) = self.tables.get_mut(cf) {
            if new_tables.is_empty() {
                // Only removing — no insertion needed
                let mut sorted_indices: Vec<usize> = indices.to_vec();
                sorted_indices.sort_unstable_by(|a, b| b.cmp(a));
                for &idx in &sorted_indices {
                    if idx < tables.len() {
                        tables.remove(idx);
                    }
                }
                return;
            }

            // The insertion point: where the first (oldest) removed table was
            let insert_at = indices.iter().min().copied().unwrap_or(0);

            // Sort indices in descending order to remove from end without invalidating indices
            let mut sorted_indices: Vec<usize> = indices.to_vec();
            sorted_indices.sort_unstable_by(|a, b| b.cmp(a));

            // Remove old tables
            for &idx in &sorted_indices {
                if idx < tables.len() {
                    tables.remove(idx);
                }
            }

            // Insert new tables at the position of the first removed table,
            // rather than appending at the end. This ensures that tables added
            // by flushes during Phase 2 (which are appended) remain AFTER the
            // compacted result, so they are checked first by `get()`'s `.rev()`.
            let insert_at = insert_at.min(tables.len());
            let _ = tables.splice(insert_at..insert_at, new_tables);
        }
    }

    /// Return statistics about the tables in a column family.
    pub fn stats(&self, cf: &str) -> VersionStats {
        let mut stats = VersionStats {
            num_tables: 0,
            total_size: 0,
            total_records: 0,
            sst_kb: 0,
            sst_files: 0,
            sst_records: 0,
            max_levels_reached: 0,
            num_tables_at_max: 0,
        };

        if let Some(cf_tables) = self.tables.get(cf) {
            stats.num_tables = cf_tables.len();
            stats.sst_files = cf_tables.len();
            stats.sst_records = cf_tables.iter().map(|t| t.data.len()).sum();
            stats.total_records = stats.sst_records;
            // Estimate size: sum of key+value lengths across all tables
            stats.total_size = cf_tables
                .iter()
                .map(|t| {
                    t.data
                        .iter()
                        .map(|(k, v)| k.len() + v.len())
                        .sum::<usize>()
                })
                .sum();
            stats.sst_kb = stats.total_size / 1024;
        }

        stats
    }

    /// Get list of all column families
    pub fn column_families(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }
}
