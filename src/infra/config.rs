use crate::infra::error::{LsmError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LsmConfig {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub dir_path: PathBuf,
    pub memtable_max_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub block_size: usize,
    pub block_cache_size_mb: usize,
    pub sparse_index_interval: usize,
    pub bloom_false_positive_rate: f64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            dir_path: PathBuf::from("./.lsmdata"),
            memtable_max_size: 4 * 1024 * 1024,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            block_size: 4096,
            block_cache_size_mb: 64,
            sparse_index_interval: 16,
            bloom_false_positive_rate: 0.01,
        }
    }
}

impl LsmConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> LsmConfigBuilder {
        LsmConfigBuilder::default()
    }

    /// Validate all configuration parameters
    pub fn validate(&self) -> Result<()> {
        self.core.validate()?;
        self.storage.validate()?;
        Ok(())
    }
}

impl CoreConfig {
    /// Validate core configuration parameters
    pub fn validate(&self) -> Result<()> {
        // Memtable size validation
        if self.memtable_max_size == 0 {
            return Err(LsmError::InvalidMemtableSize(
                "Memtable size cannot be 0".to_string(),
            ));
        }

        if self.memtable_max_size < 1024 {
            return Err(LsmError::InvalidMemtableSize(
                "Memtable size too small (minimum 1KB)".to_string(),
            ));
        }

        if self.memtable_max_size > 1024 * 1024 * 1024 {
            return Err(LsmError::InvalidMemtableSize(
                "Memtable size too large (maximum 1GB)".to_string(),
            ));
        }

        Ok(())
    }
}

impl StorageConfig {
    /// Validate storage configuration parameters
    pub fn validate(&self) -> Result<()> {
        // Block size validation
        if self.block_size == 0 {
            return Err(LsmError::InvalidBlockSize(
                "Block size cannot be 0".to_string(),
            ));
        }

        if self.block_size < 256 {
            return Err(LsmError::InvalidBlockSize(
                "Block size too small (minimum 256 bytes)".to_string(),
            ));
        }

        if self.block_size > 1024 * 1024 {
            return Err(LsmError::InvalidBlockSize(
                "Block size cannot exceed 1MB".to_string(),
            ));
        }

        // Cache size validation
        if self.block_cache_size_mb == 0 {
            return Err(LsmError::InvalidCacheSize(
                "Cache size cannot be 0".to_string(),
            ));
        }

        if self.block_cache_size_mb > 10240 {
            eprintln!(
                "⚠️  Warning: Very large cache size ({}MB), may consume excessive memory",
                self.block_cache_size_mb
            );
        }

        // Sparse index interval validation
        if self.sparse_index_interval == 0 {
            return Err(LsmError::InvalidIndexInterval(
                "Sparse index interval cannot be 0".to_string(),
            ));
        }

        if self.sparse_index_interval > 1000 {
            eprintln!(
                "⚠️  Warning: Very sparse index (interval={}), may impact read performance",
                self.sparse_index_interval
            );
        }

        // Bloom filter false positive rate validation
        if self.bloom_false_positive_rate <= 0.0 || self.bloom_false_positive_rate >= 1.0 {
            return Err(LsmError::InvalidBloomRate(
                "Bloom FP rate must be between 0 and 1 (exclusive)".to_string(),
            ));
        }

        if self.bloom_false_positive_rate > 0.1 {
            eprintln!(
                "⚠️  Warning: High Bloom filter FP rate ({}), may reduce effectiveness",
                self.bloom_false_positive_rate
            );
        }

        Ok(())
    }
}

#[derive(Default)]
pub struct LsmConfigBuilder {
    dir_path: Option<PathBuf>,
    memtable_max_size: Option<usize>,
    block_size: Option<usize>,
    block_cache_size_mb: Option<usize>,
    sparse_index_interval: Option<usize>,
    bloom_false_positive_rate: Option<f64>,
}

impl LsmConfigBuilder {
    pub fn dir_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.dir_path = Some(path.into());
        self
    }

    pub fn memtable_max_size(mut self, size: usize) -> Self {
        self.memtable_max_size = Some(size);
        self
    }

    pub fn block_size(mut self, size: usize) -> Self {
        self.block_size = Some(size);
        self
    }

    pub fn block_cache_size_mb(mut self, size: usize) -> Self {
        self.block_cache_size_mb = Some(size);
        self
    }

    pub fn sparse_index_interval(mut self, interval: usize) -> Self {
        self.sparse_index_interval = Some(interval);
        self
    }

    pub fn bloom_false_positive_rate(mut self, rate: f64) -> Self {
        self.bloom_false_positive_rate = Some(rate);
        self
    }

    pub fn build(self) -> Result<LsmConfig> {
        let defaults = LsmConfig::default();

        let config = LsmConfig {
            core: CoreConfig {
                dir_path: self.dir_path.unwrap_or(defaults.core.dir_path),
                memtable_max_size: self
                    .memtable_max_size
                    .unwrap_or(defaults.core.memtable_max_size),
            },
            storage: StorageConfig {
                block_size: self.block_size.unwrap_or(defaults.storage.block_size),
                block_cache_size_mb: self
                    .block_cache_size_mb
                    .unwrap_or(defaults.storage.block_cache_size_mb),
                sparse_index_interval: self
                    .sparse_index_interval
                    .unwrap_or(defaults.storage.sparse_index_interval),
                bloom_false_positive_rate: self
                    .bloom_false_positive_rate
                    .unwrap_or(defaults.storage.bloom_false_positive_rate),
            },
        };

        // Validate before returning
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = LsmConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_block_size_zero() {
        let config = StorageConfig {
            block_size: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBlockSize(_)));
    }

    #[test]
    fn test_invalid_block_size_too_large() {
        let config = StorageConfig {
            block_size: 2 * 1024 * 1024,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBlockSize(_)));
    }

    #[test]
    fn test_invalid_cache_size_zero() {
        let config = StorageConfig {
            block_cache_size_mb: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidCacheSize(_)));
    }

    #[test]
    fn test_invalid_index_interval_zero() {
        let config = StorageConfig {
            sparse_index_interval: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LsmError::InvalidIndexInterval(_)
        ));
    }

    #[test]
    fn test_invalid_bloom_rate_zero() {
        let config = StorageConfig {
            bloom_false_positive_rate: 0.0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBloomRate(_)));
    }

    #[test]
    fn test_invalid_bloom_rate_one() {
        let config = StorageConfig {
            bloom_false_positive_rate: 1.0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBloomRate(_)));
    }

    #[test]
    fn test_invalid_bloom_rate_negative() {
        let config = StorageConfig {
            bloom_false_positive_rate: -0.1,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBloomRate(_)));
    }

    #[test]
    fn test_invalid_memtable_size_zero() {
        let config = CoreConfig {
            memtable_max_size: 0,
            ..Default::default()
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LsmError::InvalidMemtableSize(_)
        ));
    }

    #[test]
    fn test_builder_with_validation() {
        let config = LsmConfig::builder()
            .dir_path("/tmp/test")
            .memtable_max_size(8 * 1024 * 1024)
            .block_size(8192)
            .block_cache_size_mb(128)
            .build();

        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.core.dir_path, PathBuf::from("/tmp/test"));
        assert_eq!(config.core.memtable_max_size, 8 * 1024 * 1024);
        assert_eq!(config.storage.block_size, 8192);
        assert_eq!(config.storage.block_cache_size_mb, 128);
    }

    #[test]
    fn test_builder_validation_failure() {
        let result = LsmConfig::builder()
            .block_size(0) // Invalid
            .build();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), LsmError::InvalidBlockSize(_)));
    }

    #[test]
    fn test_valid_config_range() {
        let config = LsmConfig::builder()
            .block_size(256) // Minimum
            .block_cache_size_mb(1) // Minimum
            .sparse_index_interval(1) // Minimum
            .bloom_false_positive_rate(0.001) // Small but valid
            .build();

        assert!(config.is_ok());
    }
}
