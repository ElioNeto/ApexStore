use crate::core::log_record::LogRecord;
use crate::infra::codec::encode;
use crate::infra::config::StorageConfig;
use crate::infra::error::{LsmError, Result};
use crate::storage::block::Block;
use bloomfilter::Bloom;
use lz4_flex::compress_prepend_size;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

const SST_MAGIC_V2: &[u8; 8] = b"LSMSST03";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockMeta {
    pub first_key: Vec<u8>,
    pub offset: u64,
    pub size: u32,
    pub uncompressed_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaBlock {
    pub blocks: Vec<BlockMeta>,
    pub bloom_filter_data: Vec<u8>,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
    pub record_count: u64,
    pub timestamp: u128,
}

pub struct SstableBuilder {
    writer: BufWriter<File>,
    current_block: Block,
    block_metas: Vec<BlockMeta>,
    keys_for_bloom: Vec<Vec<u8>>,
    config: StorageConfig,
    current_offset: u64,
    first_key: Option<Vec<u8>>,
    last_key: Option<Vec<u8>>,
    record_count: u64,
    path: PathBuf,
    timestamp: u128,
}

impl SstableBuilder {
    pub fn new(path: PathBuf, config: StorageConfig, timestamp: u128) -> Result<Self> {
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);

        writer.write_all(SST_MAGIC_V2)?;
        let current_offset = SST_MAGIC_V2.len() as u64;

        let current_block = Block::from_config(&config);

        Ok(Self {
            writer,
            current_block,
            block_metas: Vec::new(),
            keys_for_bloom: Vec::new(),
            config,
            current_offset,
            first_key: None,
            last_key: None,
            record_count: 0,
            path,
            timestamp,
        })
    }

    pub fn add(&mut self, key: &[u8], record: &LogRecord) -> Result<()> {
        if self.first_key.is_none() {
            self.first_key = Some(key.to_vec());
        }
        self.last_key = Some(key.to_vec());

        let value_bytes = encode(record)?;

        if !self.current_block.add(key, &value_bytes) {
            self.flush_current_block()?;

            if !self.current_block.add(key, &value_bytes) {
                return Err(LsmError::CompactionFailed(
                    "Entry too large for a single block".to_string(),
                ));
            }
        }

        self.keys_for_bloom.push(key.to_vec());
        self.record_count += 1;

        Ok(())
    }

    fn flush_current_block(&mut self) -> Result<()> {
        if self.current_block.is_empty() {
            return Ok(());
        }

        let first_key = self.extract_first_key_from_block()?;
        let encoded = self.current_block.encode();
        let uncompressed_size = encoded.len() as u32;

        let compressed = compress_prepend_size(&encoded);
        let compressed_size = compressed.len() as u32;

        self.writer.write_all(&compressed)?;

        let block_meta = BlockMeta {
            first_key,
            offset: self.current_offset,
            size: compressed_size,
            uncompressed_size,
        };

        self.block_metas.push(block_meta);
        self.current_offset += compressed_size as u64;

        self.current_block = Block::from_config(&self.config);

        Ok(())
    }

    fn extract_first_key_from_block(&self) -> Result<Vec<u8>> {
        let encoded = self.current_block.encode();
        if encoded.len() < 2 {
            return Err(LsmError::CompactionFailed(
                "Block too small to extract first key".to_string(),
            ));
        }

        let key_len = u16::from_le_bytes([encoded[0], encoded[1]]) as usize;
        if encoded.len() < 2 + key_len {
            return Err(LsmError::CompactionFailed("Corrupted block data".to_string()));
        }

        Ok(encoded[2..2 + key_len].to_vec())
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        self.flush_current_block()?;

        if self.block_metas.is_empty() {
            return Err(LsmError::CompactionFailed(
                "Cannot create SSTable with no blocks".to_string(),
            ));
        }

        let bloom = self.build_bloom_filter()?;
        let bloom_bytes = bloom.into_bytes();

        let meta_block = MetaBlock {
            blocks: self.block_metas,
            bloom_filter_data: bloom_bytes,
            min_key: self.first_key.unwrap(),
            max_key: self.last_key.unwrap(),
            record_count: self.record_count,
            timestamp: self.timestamp,
        };

        let meta_encoded = encode(&meta_block)?;
        let meta_compressed = compress_prepend_size(&meta_encoded);
        let meta_offset = self.current_offset;

        self.writer.write_all(&meta_compressed)?;

        let footer_bytes = meta_offset.to_le_bytes();
        self.writer.write_all(&footer_bytes)?;

        self.writer.flush()?;
        self.writer.get_ref().sync_all()?;

        Ok(self.path)
    }

    fn build_bloom_filter(&self) -> Result<Bloom<[u8]>> {
        let mut bloom = Bloom::<[u8]>::new_for_fp_rate(
            self.keys_for_bloom.len(),
            self.config.bloom_false_positive_rate,
        )
        .map_err(|e| LsmError::CompactionFailed(format!("Bloom filter creation failed: {}", e)))?;

        for key in &self.keys_for_bloom {
            bloom.set(key);
        }

        Ok(bloom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_record(key: &str, value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_string(), value.to_vec())
    }

    #[test]
    fn test_builder_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sst");
        let config = StorageConfig::default();

        let mut builder = SstableBuilder::new(path.clone(), config, 123).unwrap();

        builder
            .add(b"key1", &create_test_record("key1", b"value1"))
            .unwrap();
        builder
            .add(b"key2", &create_test_record("key2", b"value2"))
            .unwrap();
        builder
            .add(b"key3", &create_test_record("key3", b"value3"))
            .unwrap();

        let result_path = builder.finish().unwrap();
        assert_eq!(result_path, path);
        assert!(path.exists());
    }

    #[test]
    fn test_builder_multiple_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_multi.sst");
        let config = StorageConfig {
            block_size: 256,
            ..Default::default()
        };

        let mut builder = SstableBuilder::new(path.clone(), config, 456).unwrap();

        for i in 0..50 {
            let key = format!("key_{:03}", i);
            let value = vec![b'x'; 20];
            builder
                .add(key.as_bytes(), &create_test_record(&key, &value))
                .unwrap();
        }

        let result_path = builder.finish().unwrap();
        assert!(result_path.exists());
    }

    #[test]
    fn test_builder_empty_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.sst");
        let config = StorageConfig::default();

        let builder = SstableBuilder::new(path, config, 789).unwrap();
        let result = builder.finish();

        assert!(result.is_err());
    }

    #[test]
    fn test_builder_large_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.sst");
        let config = StorageConfig::default();

        let mut builder = SstableBuilder::new(path, config, 999).unwrap();

        let large_value = vec![b'x'; 1000];
        builder
            .add(b"large_key", &create_test_record("large_key", &large_value))
            .unwrap();

        let result = builder.finish();
        assert!(result.is_ok());
    }
}
