use crate::infra::config::StorageConfig;
use crate::storage::cache::{Cache, GlobalBlockCache};
use crate::storage::encryption::EncryptionConfig;
use crate::storage::reader::SstableReader;
use moka::sync::Cache as MokaCache;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Key-value cache for table lookups. Caches individual key-value results
    /// so repeated reads for the same key bypass table iteration.
    /// Uses `moka::sync::Cache` for concurrent, lock-free access.
    kv_cache: MokaCache<Vec<u8>, Arc<Vec<u8>>>,
    tables: std::collections::HashMap<String, Vec<crate::core::table::Table>>,
    /// Storage configuration used to open SstableReaders for on-disk tables.
    storage_config: StorageConfig,
    /// Shared block cache for SSTable block caching. `None` when no block cache
    /// is available (e.g., in tests with `NoopCache`).
    block_cache: Option<Arc<GlobalBlockCache>>,
    /// Encryption configuration for reading encrypted SSTables.
    encryption: EncryptionConfig,
    /// Monotonically increasing counter incremented every time tables are
    /// added or removed.  Background compaction plans capture this value
    /// at build time and reject their results at apply time if the counter
    /// has advanced (indicating the plan's indices are stale).
    compaction_generation: u64,
    /// Set of SSTable paths that have experienced read errors.  These tables
    /// are skipped on subsequent read attempts, and a background process
    /// moves the files out of the active directory to prevent compaction
    /// from retrying the corrupt data.
    quarantined: Arc<Mutex<HashSet<PathBuf>>>,
    /// Number of SSTables moved to quarantine during startup discovery.
    pub(crate) quarantined_count: AtomicU64,
    /// Number of SSTables recovered (data replayed from WAL) during startup.
    pub(crate) recovered_count: AtomicU64,
}

impl<C: Cache> VersionSet<C> {
    pub fn new(
        options: crate::core::engine::EngineOptions,
        _cache: C,
        storage_config: StorageConfig,
        block_cache: Option<Arc<GlobalBlockCache>>,
    ) -> Self {
        // Derive KV cache capacity from block cache size (rough estimate: entry ~200 bytes)
        let kv_capacity = ((options.block_cache_size_mb * 1024 * 1024) / 200).max(1000) as u64;
        // Build EncryptionConfig from the infra config
        let encryption = if storage_config.encryption_enabled {
            EncryptionConfig::from_key_path(storage_config.encryption_key_path.as_deref())
                .unwrap_or_default()
        } else {
            EncryptionConfig::default()
        };

        Self {
            _cache: std::marker::PhantomData,
            kv_cache: MokaCache::builder()
                .max_capacity(kv_capacity)
                .name("version_set_kv_cache")
                .build(),
            tables: std::collections::HashMap::new(),
            storage_config,
            block_cache,
            encryption,
            compaction_generation: 0,
            quarantined: Arc::new(Mutex::new(HashSet::new())),
            quarantined_count: AtomicU64::new(0),
            recovered_count: AtomicU64::new(0),
        }
    }

    /// Check if a key is cached.
    pub fn get_cached(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.kv_cache.get(key).map(|arc| (*arc).clone())
    }

    /// Store a key-value pair in the cache.
    pub fn put_cached(&self, key: Vec<u8>, value: Vec<u8>) {
        self.kv_cache.insert(key, Arc::new(value));
    }

    /// Clear the entire KV cache. Should be called after compaction or flush
    /// to prevent stale results.
    pub fn clear_cache(&self) {
        self.kv_cache.invalidate_all();
    }

    pub fn get(&self, cf: &str, key: &[u8]) -> Option<Vec<u8>> {
        // 1. Check KV cache first — avoids table iteration entirely for hot keys
        if let Some(cached) = self.get_cached(key) {
            if cached.is_empty() {
                // Empty value in cache means tombstone — key was deleted
                return None;
            }
            return Some(cached);
        }

        if let Some(cf_tables) = self.tables.get(cf) {
            'table_loop: for table in cf_tables.iter().rev() {
                // Skip tables whose key range doesn't include the target key
                if !table.min_key.is_empty()
                    && !table.max_key.is_empty()
                    && (key < table.min_key.as_slice() || key > table.max_key.as_slice())
                {
                    continue;
                }

                // Use cached bloom filter to avoid I/O
                if let Some(ref bloom_filter) = table.bloom_filter {
                    if !bloom_filter.check(key) {
                        // Bloom filter says key definitely does not exist -> skip
                        continue 'table_loop;
                    }
                    // Bloom says key might exist, fall through to BTreeMap lookup
                }

                // Check in-memory data first
                if let Some(val) = table.data.get(key) {
                    if val.is_empty() {
                        // No on-disk SSTable to fall back to:
                        // empty value means tombstone.
                        table.path.as_ref()?;
                        // Has a path: fall through to the SSTable reader
                        // which correctly distinguishes tombstones from
                        // legitimate empty values via the is_deleted flag.
                    } else {
                        // Non-empty value: populate cache and return
                        self.put_cached(key.to_vec(), val.clone());
                        return Some(val.clone());
                    }
                }

                // 3. If not in memory but has a disk path, try reading from SSTable
                if let Some(ref path) = table.path {
                    // Skip tables that have been quarantined due to prior read errors
                    if self.quarantined.lock().contains(path) {
                        continue 'table_loop;
                    }

                    if let Some(ref block_cache) = self.block_cache {
                        match SstableReader::open_with_encryption(
                            path.clone(),
                            self.storage_config.clone(),
                            block_cache.clone(),
                            &self.encryption,
                        ) {
                            Ok(reader) => match reader.get(key) {
                                Ok(Some(record)) => {
                                    // Tombstone: SSTable reader sets is_deleted flag
                                    if record.is_deleted {
                                        // Tombstone → key is deleted, stop searching
                                        return None;
                                    }
                                    // TTL expiry: key was stored with an expiration time
                                    // in the SSTable's LogRecord metadata.
                                    if record.is_expired() {
                                        // Key has expired — treat as not found
                                        continue 'table_loop;
                                    }
                                    let value = record.value;
                                    self.put_cached(key.to_vec(), value.clone());
                                    return Some(value);
                                }
                                // Not found in this SSTable — continue to next table
                                Ok(None) => continue 'table_loop,
                                // I/O or corruption error — quarantine this SSTable
                                Err(e) => {
                                    tracing::warn!(
                                        target: "apexstore::quarantine",
                                        path = %path.display(),
                                        error = %e,
                                        "SSTable read error — quarantining file"
                                    );
                                    self.quarantined.lock().insert(path.clone());
                                    continue 'table_loop;
                                }
                            },
                            // Can't open reader — quarantine this SSTable
                            Err(e) => {
                                tracing::warn!(
                                    target: "apexstore::quarantine",
                                    path = %path.display(),
                                    error = %e,
                                    "SSTable open error — quarantining file"
                                );
                                self.quarantined.lock().insert(path.clone());
                                continue 'table_loop;
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Returns `true` if the given SSTable path has been quarantined.
    pub fn is_quarantined(&self, path: &PathBuf) -> bool {
        self.quarantined.lock().contains(path)
    }

    /// Move all quarantined SSTable files out of the active SSTable directory
    /// into a quarantine subdirectory so compaction and future reads avoid them.
    ///
    /// Returns the number of files successfully moved.
    pub fn evacuate_quarantined(&self, sst_dir: &std::path::Path) -> usize {
        let paths: Vec<PathBuf> = self.quarantined.lock().iter().cloned().collect();
        let quarantine_dir = sst_dir.join("quarantine");
        let _ = std::fs::create_dir_all(&quarantine_dir);
        let mut moved = 0;
        for path in &paths {
            let dest = quarantine_dir.join(
                path.file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
            );
            match std::fs::rename(path, &dest) {
                Ok(()) => {
                    tracing::info!(
                        target: "apexstore::quarantine",
                        from = %path.display(),
                        to = %dest.display(),
                        "Quarantined SSTable moved"
                    );
                    moved += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "apexstore::quarantine",
                        path = %path.display(),
                        error = %e,
                        "Failed to move quarantined SSTable"
                    );
                }
            }
        }
        self.quarantined.lock().clear();
        moved
    }

    /// Return the number of currently quarantined SSTables.
    pub fn quarantined_count(&self) -> usize {
        self.quarantined.lock().len()
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
        // New table means previously cached entries might have been superseded
        self.clear_cache();
        self.compaction_generation += 1;
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
        let result = self.tables.remove(cf).unwrap_or_default();
        self.clear_cache();
        result
    }

    pub fn remove_and_add_table(&mut self, cf: &str, new_table: crate::core::table::Table) {
        // Remove todas as tabelas da CF e substitui pela tabela compactada.
        let entry = self.tables.entry(cf.to_string()).or_default();
        entry.clear();
        entry.push(new_table);
        self.compaction_generation += 1;
    }

    /// Get all tables for a column family (without draining)
    pub fn get_tables(&self, cf: &str) -> Vec<crate::core::table::Table> {
        self.tables.get(cf).map_or_else(Vec::new, |v| v.clone())
    }

    /// Atomically replace specific tables with new ones.
    ///
    /// Returns the list of old SSTable file paths that were removed, so the
    /// caller can clean up orphaned `.sst` files from disk.
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
    ) -> Vec<std::path::PathBuf> {
        let mut removed_paths = Vec::new();
        if let Some(tables) = self.tables.get_mut(cf) {
            if new_tables.is_empty() {
                // Only removing — no insertion needed
                let mut sorted_indices: Vec<usize> = indices.to_vec();
                sorted_indices.sort_unstable_by(|a, b| b.cmp(a));
                for &idx in &sorted_indices {
                    if idx < tables.len() {
                        if let Some(ref path) = tables[idx].path {
                            removed_paths.push(path.clone());
                        }
                        tables.remove(idx);
                    }
                }
                return removed_paths;
            }

            // Record old table paths before removal
            for &idx in indices {
                if idx < tables.len() {
                    if let Some(ref path) = tables[idx].path {
                        removed_paths.push(path.clone());
                    }
                }
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
            self.compaction_generation += 1;
        }
        removed_paths
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
                .map(|t| t.data.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>())
                .sum();
            stats.sst_kb = stats.total_size / 1024;
        }

        stats
    }

    /// Get list of all column families
    pub fn column_families(&self) -> Vec<String> {
        self.tables.keys().cloned().collect()
    }

    /// Current compaction generation.  Stale-plan detection:
    /// capture this before building a plan, and compare when applying results.
    pub fn compaction_generation(&self) -> u64 {
        self.compaction_generation
    }

    /// Number of SSTables quarantined during startup discovery.
    pub fn startup_quarantine_count(&self) -> u64 {
        self.quarantined_count.load(Ordering::Relaxed)
    }

    /// Number of SSTables recovered (data replayed from WAL) during startup.
    /// Currently only counted — WAL replay for explicit recovery is a future enhancement.
    pub fn recovered_count(&self) -> u64 {
        self.recovered_count.load(Ordering::Relaxed)
    }
}
