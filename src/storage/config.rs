use crate::storage::encryption::EncryptionConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompactionStrategy {
    SizeTiered,
    Leveled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub block_size: usize,
    pub block_cache_size_mb: usize,
    pub sparse_index_interval: usize,
    pub compaction_strategy: CompactionStrategy,
    pub bloom_false_positive_rate: f64,
    /// Encryption configuration (disabled by default).
    #[serde(default)]
    pub encryption: EncryptionConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            block_size: 4096,
            block_cache_size_mb: 64,
            sparse_index_interval: 16,
            compaction_strategy: CompactionStrategy::SizeTiered,
            bloom_false_positive_rate: 0.01,
            encryption: EncryptionConfig::default(),
        }
    }
}
