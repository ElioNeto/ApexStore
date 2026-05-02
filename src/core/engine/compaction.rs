use crate::core::engine::EngineOptions;
use crate::core::iterators::{MergeIterator, StorageIterator};
use crate::core::key::KeySlice;
use crate::core::table::Table;
use crate::infra::error::{LsmError, Result};
use crate::storage::builder::SstableBuilder;
use crate::storage::config::StorageConfig;
use std::collections::BTreeMap;
use std::path::PathBuf;
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
pub trait CompactionStrategy: Send + Sync {
    /// Pick tables that should be compacted.
    /// Returns a vector of groups, where each group is a vector of tables to merge together.
    fn pick_tables(&self, tables: &[Table], options: &EngineOptions) -> Vec<Vec<usize>>;

    /// Execute compaction on the given tables and return new tables.
    fn execute(
        &self,
        tables: Vec<Table>,
        options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &PathBuf,
    ) -> Result<(Vec<Table>, CompactionMetrics)>;

    /// Returns the name of the strategy.
    fn name(&self) -> &'static str;
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
        options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &PathBuf,
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        let start_time = SystemTime::now();
        let mut metrics = CompactionMetrics::default();
        metrics.files_merged = tables.len();

        if tables.is_empty() {
            return Ok((Vec::new(), metrics));
        }

        // Calculate bytes read
        for table in &tables {
            metrics.bytes_read += Self::get_table_size(table) as u64;
        }

        // Merge tables using MergeIterator
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
        for table in &tables {
            iters.push(Box::new(table.iter()));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Create output SSTable
        let output_path = output_dir.join(format!("sst_{}.sst", timestamp));
        let mut builder = SstableBuilder::new(output_path.clone(), storage_config.clone(), timestamp)?;

        let mut record_count = 0u64;
        while merge_iter.is_valid() {
            let key = merge_iter.key();
            let value = merge_iter.value();

            // Skip tombstones (empty values) during compaction
            if !value.is_empty() {
                use crate::core::log_record::LogRecord;
                let record = LogRecord::new(key.as_ref().to_vec(), value.to_vec());
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
        // For now, we'll read it back - in production this would be more efficient
        let new_table = Table::from_sstable_path(&result_path)?;

        // Update duration
        metrics.duration_ms = SystemTime::now()
            .duration_since(start_time)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok((vec![new_table], metrics))
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
            10 * 1024 * 1024,    // L1: 10MB
            100 * 1024 * 1024,   // L2: 100MB
            1024 * 1024 * 1024,  // L3: 1GB
        ];
        Self {
            level_multiplier: 10,
            max_level_size,
        }
    }
}

impl LeveledCompaction {
    fn get_table_size(table: &Table) -> usize {
        table
            .data
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
    }
}

impl CompactionStrategy for LeveledCompaction {
    fn pick_tables(&self, tables: &[Table], options: &EngineOptions) -> Vec<Vec<usize>> {
        // Simplified: pick L0 tables that exceed threshold
        let l0_tables: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.level == 0)
            .map(|(i, _)| i)
            .collect();

        if l0_tables.len() >= options.compaction_options.compaction_threshold {
            vec![l0_tables]
        } else {
            Vec::new()
        }
    }

    fn execute(
        &self,
        tables: Vec<Table>,
        options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &PathBuf,
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        let start_time = SystemTime::now();
        let mut metrics = CompactionMetrics::default();
        metrics.files_merged = tables.len();

        if tables.is_empty() {
            return Ok((Vec::new(), metrics));
        }

        // Calculate bytes read
        for table in &tables {
            metrics.bytes_read += Self::get_table_size(table) as u64;
        }

        // Merge tables using MergeIterator
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();
        for table in &tables {
            iters.push(Box::new(table.iter()));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Create output SSTable at L1 (or next level)
        let output_path = output_dir.join(format!("sst_L1_{}.sst", timestamp));
        let mut builder = SstableBuilder::new(output_path.clone(), storage_config.clone(), timestamp)?;

        let mut record_count = 0u64;
        while merge_iter.is_valid() {
            let key = merge_iter.key();
            let value = merge_iter.value();

            // Skip tombstones during compaction
            if !value.is_empty() {
                use crate::core::log_record::LogRecord;
                let record = LogRecord::new(key.as_ref().to_vec(), value.to_vec());
                builder.add(key.as_ref(), &record)?;
                record_count += 1;
            }

            merge_iter.next();
        }

        if record_count == 0 {
            return Ok((Vec::new(), metrics));
        }

        let result_path = builder.finish()?;
        metrics.bytes_written = std::fs::metadata(&result_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Create new Table from the SSTable at level 1
        let mut new_table = Table::from_sstable_path(&result_path)?;
        new_table.level = 1; // Promote to L1

        // Update duration
        metrics.duration_ms = SystemTime::now()
            .duration_since(start_time)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok((vec![new_table], metrics))
    }

    fn name(&self) -> &'static str {
        "Leveled"
    }
}

/// Lazy Leveling Compaction Strategy
///
/// Hybrid approach: top level (L0) uses Size-Tiered, lower levels use Leveled.
/// This reduces write amplification compared to pure Leveled.
pub struct LazyLevelingCompaction {
    pub size_tiered: SizeTieredCompaction,
    pub leveled: LeveledCompaction,
}

impl Default for LazyLevelingCompaction {
    fn default() -> Self {
        Self {
            size_tiered: SizeTieredCompaction::default(),
            leveled: LeveledCompaction::default(),
        }
    }
}

impl CompactionStrategy for LazyLevelingCompaction {
    fn pick_tables(&self, tables: &[Table], options: &EngineOptions) -> Vec<Vec<usize>> {
        // L0 uses size-tiered, lower levels use leveled
        let l0_tables: Vec<usize> = tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.level == 0)
            .map(|(i, _)| i)
            .collect();

        let l0_indices: Vec<usize> = l0_tables.iter().copied().collect();
        
        if !l0_indices.is_empty() {
            // Use size-tiered for L0
            let l0_tables_ref: Vec<Table> = l0_indices
                .iter()
                .map(|&i| tables[i].clone())
                .collect();
            
            let buckets = SizeTieredCompaction::group_into_buckets(&l0_tables_ref, self.size_tiered.min_tables_to_merge);
            
            // Map back to original indices
            buckets
                .into_iter()
                .map(|bucket| {
                    bucket
                        .iter()
                        .map(|&local_idx| l0_indices[local_idx])
                        .collect()
                })
                .collect()
        } else {
            // Use leveled for lower levels
            self.leveled.pick_tables(tables, options)
        }
    }

    fn execute(
        &self,
        tables: Vec<Table>,
        options: &EngineOptions,
        storage_config: &StorageConfig,
        output_dir: &PathBuf,
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        // Determine which strategy to use based on table levels
        let has_l0 = tables.iter().any(|t| t.level == 0);
        
        if has_l0 {
            self.size_tiered.execute(tables, options, storage_config, output_dir)
        } else {
            self.leveled.execute(tables, options, storage_config, output_dir)
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
            crate::infra::config::CompactionStrategy::SizeTiered => CompactionStrategyType::SizeTiered,
            crate::infra::config::CompactionStrategy::Leveled => CompactionStrategyType::Leveled,
            crate::infra::config::CompactionStrategy::LazyLeveling => CompactionStrategyType::LazyLeveling,
        }
    }
}

impl From<crate::infra::config::CompactionStrategy> for CompactionOptions {
    fn from(config: crate::infra::config::CompactionStrategy) -> Self {
        let strategy_type: CompactionStrategyType = config.into();
        CompactionOptions {
            strategy_type,
            compaction_threshold: 4, // default
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
        let strategy_type: CompactionStrategyType = config.compaction.strategy.into();
        let options = CompactionOptions {
            strategy_type,
            compaction_threshold: config.compaction.min_compaction_threshold,
            max_tables_per_compaction: config.compaction.max_sstables,
        };
        let storage_config = crate::storage::config::StorageConfig {
            block_size: config.storage.block_size,
            sparse_index_interval: config.storage.sparse_index_interval,
            bloom_false_positive_rate: config.storage.bloom_false_positive_rate,
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
        table_indices: Vec<usize>,
        all_tables: &[Table],
        options: &EngineOptions,
    ) -> Result<(Vec<Table>, CompactionMetrics)> {
        let tables: Vec<Table> = table_indices
            .into_iter()
            .map(|i| all_tables[i].clone())
            .collect();

        self.strategy
            .execute(tables, options, &self.storage_config, &self.output_dir)
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
