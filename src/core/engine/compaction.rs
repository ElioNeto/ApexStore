use crate::core::engine::EngineOptions;
use crate::core::iterators::{MergeIterator, StorageIterator};
use crate::core::key::KeySlice;
use crate::core::log_record::{LogRecord, RangeTombstone};
use crate::core::table::Table;
use crate::infra::config::StorageConfig;
use crate::infra::error::Result;
use crate::storage::builder::SstableBuilder;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Metrics collected during a compaction operation.
#[derive(Debug, Clone, Default)]
pub struct CompactionMetrics {
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub files_merged: usize,
    pub duration_ms: u64,
}

/// Trait for compaction strategies.
///
/// Implementations define how SSTables are grouped and merged to maintain
/// read/write performance and bound space amplification.
///
/// # Usage example
///
/// ```rust
/// use apexstore::core::engine::compaction::{
///     CompactionStrategy, SizeTieredCompaction, CompactionMetrics,
/// };
/// use apexstore::core::engine::EngineOptions;
/// use apexstore::infra::config::StorageConfig;
/// use apexstore::core::table::Table;
/// use std::collections::BTreeMap;
///
/// let strategy = SizeTieredCompaction::default();
/// let options = EngineOptions::default();
/// let storage = StorageConfig::default();
/// let dir = tempfile::tempdir().unwrap();
///
/// // Build a single table with some data
/// let mut data = BTreeMap::new();
/// data.insert(b"a".to_vec(), b"1".to_vec());
/// let table = Table::build(data, &options);
///
/// let output_dir = dir.path().to_path_buf();
/// let (new_tables, metrics) = strategy
///     .execute(vec![table], &options, &storage, &output_dir, &[])
///     .unwrap();
///
/// assert!(!new_tables.is_empty());
/// assert!(metrics.bytes_read > 0);
/// ```
pub trait CompactionStrategy: Send + Sync {
    /// Pick tables that should be compacted.
    /// Returns a vector of groups, where each group is a vector of tables to merge together.
    fn pick_tables(&self, tables: &[Table], options: &EngineOptions) -> Vec<Vec<usize>>;

    /// Execute compaction on the given tables and return new tables.
    ///
    /// `range_tombstones` is the list of active range tombstones that should be
    /// applied during compaction (keys falling within any range tombstone are dropped).
    fn execute(
        &self,
        tables: Vec<Table>,
        options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &Path,
        range_tombstones: &[RangeTombstone],
    ) -> Result<(Vec<Table>, CompactionMetrics)>;

    /// Returns the name of the strategy.
    fn name(&self) -> &'static str;
}

/// Check if a key falls within any of the given range tombstones.
fn is_key_in_range_tombstones(key: &[u8], tombstones: &[RangeTombstone]) -> bool {
    tombstones
        .iter()
        .any(|rt| rt.start_key.as_slice() <= key && key < rt.end_key.as_slice())
}

/// Shared helper for compaction execution logic
///
/// NOTE: TTL / `expires_at` metadata is not available at compaction time
/// because `Table` stores only raw `(Vec<u8>, Vec<u8>)` pairs — the
/// `LogRecord` metadata is stripped during `flush_memtable_impl()`.
/// Expired keys are therefore filtered **before** they reach the SSTable
/// (in `flush_memtable_impl`).  Compaction itself does not re-check TTL.
///
/// If TTL-awareness is needed at the compaction layer in the future, the
/// `Table` / SSTable format will need to carry expiration metadata.
fn execute_compaction(
    tables: &[Table],
    storage_config: &StorageConfig,
    output_dir: &Path,
    output_prefix: &str,
    level: Option<usize>,
    range_tombstones: &[RangeTombstone],
) -> Result<(Vec<Table>, CompactionMetrics)> {
    let start_time = SystemTime::now();
    let mut metrics = CompactionMetrics {
        files_merged: tables.len(),
        ..Default::default()
    };

    if tables.is_empty() {
        return Ok((Vec::new(), metrics));
    }

    // Calculate bytes read
    for table in tables {
        metrics.bytes_read += table.size() as u64;
    }

    // Merge tables using MergeIterator
    let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
    for table in tables {
        iters.push(Box::new(table.iter()));
    }

    let mut merge_iter = MergeIterator::new(iters);
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();

    // Create output SSTable — use encrypted builder if encryption is enabled
    let output_path = output_dir.join(format!("{}_{}.sst", output_prefix, timestamp));
    let mut builder = SstableBuilder::new_with_encryption(
        output_path.clone(),
        storage_config.clone(),
        timestamp,
        &storage_config.encryption,
    )?;

    let mut record_count = 0u64;
    while merge_iter.is_valid() {
        let key = merge_iter.key();
        let value = merge_iter.value();

        // Tombstone convention: deleted keys are stored with an empty value
        // (Vec<u8> of length 0) throughout the system.  All paths — memtable
        // flush, compaction, and point lookups — treat `is_empty()` as the
        // tombstone signal.  This avoids carrying a separate boolean per
        // record in the SSTable format while keeping tombstone detection
        // cheap (a single length check).
        //
        // During compaction, tombstones are dropped entirely: the deleted key
        // no longer appears in the compacted output since it cannot affect
        // future reads (a later tombstone overriding an earlier value would
        // be resolved the same way — dropped).
        // Skip tombstones (empty values) during compaction
        if !value.is_empty() {
            // Apply range tombstones: skip keys that fall within a range tombstone
            if is_key_in_range_tombstones(key.as_slice(), range_tombstones) {
                merge_iter.next();
                continue;
            }
            let key_vec: Vec<u8> = key.as_slice().to_vec();
            let record = LogRecord::new(key_vec, value.to_vec());
            builder.add(key.as_ref(), &record)?;
            record_count += 1;
        }

        merge_iter.next();
    }

    if record_count == 0 {
        // All data was tombstones, no output
        return Ok((Vec::new(), metrics));
    }

    let result_path = builder.finish()?;
    metrics.bytes_written = std::fs::metadata(&result_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Create new Table from the SSTable
    let mut new_table =
        Table::from_sstable_path(&result_path, Some(&storage_config.encryption))?;
    if let Some(lvl) = level {
        new_table.level = lvl;
    }

    // Update duration
    metrics.duration_ms = SystemTime::now()
        .duration_since(start_time)
        .unwrap_or_default()
        .as_millis() as u64;

    Ok((vec![new_table], metrics))
}

/// Size-Tiered Compaction Strategy
///
/// Groups tables by size into buckets. When a bucket reaches a threshold,
/// all tables in that bucket are merged together.
/// Write amplification: < 3x
pub struct SizeTieredCompaction {
    pub bucket_threshold: usize,
    pub min_tables_to_merge: usize,
}

impl Default for SizeTieredCompaction {
    fn default() -> Self {
        Self {
            bucket_threshold: 4,
            min_tables_to_merge: 2,
        }
    }
}

impl SizeTieredCompaction {
    fn get_table_size(table: &Table) -> usize {
        // Estimate size based on data
        table
            .data
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
    }

    fn group_into_buckets(tables: &[Table], bucket_threshold: usize) -> Vec<Vec<usize>> {
        let mut buckets: Vec<(usize, Vec<usize>)> = Vec::new(); // (size, indices)

        for (idx, table) in tables.iter().enumerate() {
            let size = Self::get_table_size(table);
            let bucket_size = if size < 64 * 1024 {
                64 * 1024 // < 64KB
            } else if size < 256 * 1024 {
                256 * 1024 // < 256KB
            } else if size < 1024 * 1024 {
                1024 * 1024 // < 1MB
            } else {
                usize::MAX // >= 1MB
            };

            if let Some(bucket) = buckets.iter_mut().find(|(bs, _)| *bs == bucket_size) {
                bucket.1.push(idx);
            } else {
                buckets.push((bucket_size, vec![idx]));
            }
        }

        buckets
            .into_iter()
            .filter(|(_, indices)| indices.len() >= bucket_threshold)
            .map(|(_, indices)| indices)
            .collect()
    }
}

impl CompactionStrategy for SizeTieredCompaction {
    fn pick_tables(&self, tables: &[Table], _options: &EngineOptions) -> Vec<Vec<usize>> {
        Self::group_into_buckets(tables, self.min_tables_to_merge)
    }

    fn execute(
        &self,
        tables: Vec<Table>,
        _options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &Path,
        range_tombstones: &[RangeTombstone],
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        execute_compaction(&tables, storage_config, output_dir, "sst", None, range_tombstones)
    }

    fn name(&self) -> &'static str {
        "SizeTiered"
    }
}

/// Leveled Compaction Strategy
///
/// Organizes tables into levels (L0, L1, L2...).
/// Each level is 10x larger than the previous.
/// L0 tables can overlap, but lower levels have non-overlapping key ranges.
/// Write amplification: < 10x
pub struct LeveledCompaction {
    pub level_multiplier: usize,
    pub max_level_size: Vec<usize>,
}

impl Default for LeveledCompaction {
    fn default() -> Self {
        // Level sizes: L1=10MB, L2=100MB, L3=1GB, etc.
        let max_level_size = vec![
            10 * 1024 * 1024,   // L1: 10MB
            100 * 1024 * 1024,  // L2: 100MB
            1024 * 1024 * 1024, // L3: 1GB
        ];
        Self {
            level_multiplier: 10,
            max_level_size,
        }
    }
}

impl LeveledCompaction {
    #[allow(dead_code)]
    fn get_table_size(table: &Table) -> usize {
        table
            .data
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
    }
}

impl CompactionStrategy for LeveledCompaction {
    fn pick_tables(&self, tables: &[Table], _options: &EngineOptions) -> Vec<Vec<usize>> {
        // Simplified: pick L0 tables that exceed threshold
        let l0_tables: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.level == 0)
            .map(|(i, _)| i)
            .collect();

        // Use self.level_multiplier to determine threshold
        if l0_tables.len() >= self.level_multiplier {
            vec![l0_tables]
        } else {
            Vec::new()
        }
    }

    fn execute(
        &self,
        tables: Vec<Table>,
        _options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &Path,
        range_tombstones: &[RangeTombstone],
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        execute_compaction(
            &tables,
            storage_config,
            output_dir,
            "sst_L1",
            Some(1),
            range_tombstones,
        )
    }

    fn name(&self) -> &'static str {
        "Leveled"
    }
}

/// Lazy Leveling Compaction Strategy
///
/// Hybrid approach: top level (L0) uses Size-Tiered, lower levels use Leveled.
/// This reduces write amplification compared to pure Leveled.
#[derive(Default)]
pub struct LazyLevelingCompaction {
    pub size_tiered: SizeTieredCompaction,
    pub leveled: LeveledCompaction,
}

impl CompactionStrategy for LazyLevelingCompaction {
    fn pick_tables(&self, tables: &[Table], _options: &EngineOptions) -> Vec<Vec<usize>> {
        // L0 uses size-tiered, lower levels use leveled
        let l0_tables: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.level == 0)
            .map(|(i, _)| i)
            .collect();

        let l0_indices: Vec<usize> = l0_tables.to_vec();

        if !l0_indices.is_empty() {
            // Use size-tiered for L0
            let l0_tables_ref: Vec<Table> = l0_indices.iter().map(|&i| tables[i].clone()).collect();

            let buckets = SizeTieredCompaction::group_into_buckets(
                &l0_tables_ref,
                self.size_tiered.min_tables_to_merge,
            );

            // Map back to original indices (with bounds check)
            buckets
                .into_iter()
                .map(|bucket| {
                    bucket
                        .iter()
                        .filter(|&&local_idx| local_idx < l0_indices.len())
                        .map(|&local_idx| l0_indices[local_idx])
                        .collect()
                })
                .collect()
        } else {
            // Use leveled for lower levels
            self.leveled.pick_tables(tables, _options)
        }
    }

    fn execute(
        &self,
        tables: Vec<Table>,
        _options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &Path,
        range_tombstones: &[RangeTombstone],
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        // Determine which strategy to use based on table levels
        let has_l0 = tables.iter().any(|t| t.level == 0);

        if has_l0 {
            self.size_tiered
                .execute(tables, _options, storage_config, output_dir, range_tombstones)
        } else {
            self.leveled
                .execute(tables, _options, storage_config, output_dir, range_tombstones)
        }
    }

    fn name(&self) -> &'static str {
        "LazyLeveling"
    }
}

/// Compaction configuration
#[derive(Debug, Clone)]
pub struct CompactionOptions {
    pub strategy_type: CompactionStrategyType,
    pub compaction_threshold: usize,
    pub max_tables_per_compaction: usize,
}

impl Default for CompactionOptions {
    fn default() -> Self {
        Self {
            strategy_type: CompactionStrategyType::SizeTiered,
            compaction_threshold: 4,
            max_tables_per_compaction: 8,
        }
    }
}

/// Type of compaction strategy to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStrategyType {
    SizeTiered,
    Leveled,
    LazyLeveling,
}

impl From<crate::infra::config::CompactionStrategy> for CompactionStrategyType {
    fn from(s: crate::infra::config::CompactionStrategy) -> Self {
        match s {
            crate::infra::config::CompactionStrategy::SizeTiered => {
                CompactionStrategyType::SizeTiered
            }
            crate::infra::config::CompactionStrategy::Leveled => CompactionStrategyType::Leveled,
            crate::infra::config::CompactionStrategy::LazyLeveling => {
                CompactionStrategyType::LazyLeveling
            }
        }
    }
}

impl From<crate::infra::config::CompactionStrategy> for CompactionOptions {
    fn from(config: crate::infra::config::CompactionStrategy) -> Self {
        let strategy_type: CompactionStrategyType = config.into();
        CompactionOptions {
            strategy_type,
            compaction_threshold: 4,      // default
            max_tables_per_compaction: 8, // default
        }
    }
}

/// Main compaction struct that uses the strategy pattern
pub struct Compaction {
    strategy: Box<dyn CompactionStrategy>,
    options: CompactionOptions,
    storage_config: StorageConfig,
    output_dir: PathBuf,
}

impl Compaction {
    /// Get reference to compaction options
    pub fn options(&self) -> &CompactionOptions {
        &self.options
    }
}

impl Clone for Compaction {
    fn clone(&self) -> Self {
        // Note: Cloning will use default strategy since we can't clone dyn trait objects
        // In practice, Compaction is created via new() or from_config()
        let strategy: Box<dyn CompactionStrategy> = match self.options.strategy_type {
            CompactionStrategyType::SizeTiered => Box::new(SizeTieredCompaction::default()),
            CompactionStrategyType::Leveled => Box::new(LeveledCompaction::default()),
            CompactionStrategyType::LazyLeveling => Box::new(LazyLevelingCompaction::default()),
        };

        Self {
            strategy,
            options: self.options.clone(),
            storage_config: self.storage_config.clone(),
            output_dir: self.output_dir.clone(),
        }
    }
}

impl std::fmt::Debug for Compaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compaction")
            .field("strategy", &self.strategy.name())
            .field("options", &self.options)
            .field("output_dir", &self.output_dir)
            .finish()
    }
}

impl Compaction {
    pub fn new(
        strategy_type: CompactionStrategyType,
        options: CompactionOptions,
        storage_config: StorageConfig,
        output_dir: PathBuf,
    ) -> Self {
        let strategy: Box<dyn CompactionStrategy> = match strategy_type {
            CompactionStrategyType::SizeTiered => Box::new(SizeTieredCompaction::default()),
            CompactionStrategyType::Leveled => Box::new(LeveledCompaction::default()),
            CompactionStrategyType::LazyLeveling => Box::new(LazyLevelingCompaction::default()),
        };

        Self {
            strategy,
            options,
            storage_config,
            output_dir,
        }
    }

    pub fn from_config(config: &crate::infra::config::LsmConfig, output_dir: PathBuf) -> Self {
        let strategy_type: CompactionStrategyType = config.compaction.strategy.clone().into();
        let options = CompactionOptions {
            strategy_type,
            compaction_threshold: config.compaction.min_compaction_threshold,
            max_tables_per_compaction: config.compaction.max_sstables,
        };
        let storage_config = crate::infra::config::StorageConfig {
            block_size: config.storage.block_size,
            block_cache_size_mb: config.storage.block_cache_size_mb,
            sparse_index_interval: config.storage.sparse_index_interval,
            bloom_false_positive_rate: config.storage.bloom_false_positive_rate,
            encryption_enabled: config.storage.encryption_enabled,
            encryption_key_path: config.storage.encryption_key_path.clone(),
        };

        Self::new(strategy_type, options, storage_config, output_dir)
    }

    /// Pick tables that should be compacted
    pub fn pick_compaction(&self, tables: &[Table], options: &EngineOptions) -> Vec<Vec<usize>> {
        self.strategy.pick_tables(tables, options)
    }

    /// Execute compaction on the given tables
    pub fn compact(
        &self,
        table_indices: &[usize],
        all_tables: &[Table],
        options: &EngineOptions,
        range_tombstones: &[RangeTombstone],
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        // Defensive bounds check: skip indices out of range to avoid panics
        // from off-by-one errors in group index selection.
        let tables: Vec<Table> = table_indices
            .iter()
            .filter(|&&i| i < all_tables.len())
            .map(|i| all_tables[*i].clone())
            .collect();

        if tables.is_empty() {
            return Ok((Vec::new(), CompactionMetrics::default()));
        }

        self.strategy
            .execute(tables, options, &self.storage_config, &self.output_dir, range_tombstones)
    }

    /// Get the strategy name
    pub fn strategy_name(&self) -> &'static str {
        self.strategy.name()
    }
}

impl Default for Compaction {
    fn default() -> Self {
        let options = CompactionOptions::default();
        Self::new(
            options.strategy_type,
            options,
            StorageConfig::default(),
            PathBuf::from("/tmp/sstables"),
        )
    }
}
