use crate::infra::{config::StorageConfig, error::LsmError};
use crc32fast::Hasher;
use std::mem::size_of;

pub const BLOCK_SIZE: usize = 4096;
const U32_SIZE: usize = size_of::<u32>();

#[derive(Debug, Clone)]
pub struct Block {
    pub(crate) data: Vec<u8>,
    pub(crate) offsets: Vec<u32>,
    block_size: usize,
}

impl Block {
    pub fn from_config(config: &StorageConfig) -> Self {
        Self::new(config.block_size)
    }

    pub fn new(block_size: usize) -> Self {
        Self {
            data: Vec::new(),
            offsets: Vec::new(),
            block_size,
        }
    }

    fn entry_size(key: &[u8], value: &[u8]) -> usize {
        // KeyLen(2) + Key + ValLen(2) + Value
        2 + key.len() + 2 + value.len()
    }

    fn metadata_size(num_entries: usize) -> usize {
        (num_entries * U32_SIZE) + U32_SIZE
    }

    fn current_size(&self) -> usize {
        self.data.len() + Self::metadata_size(self.offsets.len()) + U32_SIZE
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) -> bool {
        let entry_size = Self::entry_size(key, value);
        let new_offset_size = U32_SIZE;
        let total_needed = self.current_size() + entry_size + new_offset_size;

        if total_needed > self.block_size {
            return false;
        }

        let offset = self.data.len() as u32;
        self.offsets.push(offset);

        // Cast to u16 is safe for key/value lengths as we assume
        // individual entries don't exceed 64KB, even if the block does.
        let key_len = key.len() as u16;
        let val_len = value.len() as u16;

        self.data.extend_from_slice(&key_len.to_le_bytes());
        self.data.extend_from_slice(key);
        self.data.extend_from_slice(&val_len.to_le_bytes());
        self.data.extend_from_slice(value);

        true
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(self.current_size());
        encoded.extend_from_slice(&self.data);

        for &offset in &self.offsets {
            encoded.extend_from_slice(&offset.to_le_bytes());
        }

        let num_elements = self.offsets.len() as u32;
        encoded.extend_from_slice(&num_elements.to_le_bytes());

        // Calculate and append CRC32 checksum (Little Endian)
        let mut hasher = Hasher::new();
        hasher.update(&encoded);
        let checksum = hasher.finalize();
        encoded.extend_from_slice(&checksum.to_le_bytes());

        encoded
    }

    pub fn decode(data: &[u8]) -> std::result::Result<Self, LsmError> {
        if data.len() < 2 * U32_SIZE {
            return Err(LsmError::CorruptedData(
                "Data too short to contain checksum".to_string(),
            ));
        }

        // Read stored checksum (last 4 bytes)
        let checksum_start = data.len() - U32_SIZE;
        let stored_checksum = u32::from_le_bytes([
            data[checksum_start],
            data[checksum_start + 1],
            data[checksum_start + 2],
            data[checksum_start + 3],
        ]);

        // Extract data without checksum for verification
        let data_without_checksum = &data[..checksum_start];

        // Calculate actual checksum
        let mut hasher = Hasher::new();
        hasher.update(data_without_checksum);
        let calculated_checksum = hasher.finalize();

        // Verify checksum
        if stored_checksum != calculated_checksum {
            return Err(LsmError::CorruptedData(
                "CRC32 checksum mismatch: data corruption detected".to_string(),
            ));
        }

        let num_elements_start = data_without_checksum.len() - U32_SIZE;
        let num_elements = u32::from_le_bytes([
            data_without_checksum[num_elements_start],
            data_without_checksum[num_elements_start + 1],
            data_without_checksum[num_elements_start + 2],
            data_without_checksum[num_elements_start + 3],
        ]) as usize;

        let offsets_start = data_without_checksum.len() - U32_SIZE - (num_elements * U32_SIZE);
        let records_data = data_without_checksum[..offsets_start].to_vec();

        let mut offsets = Vec::with_capacity(num_elements);
        let mut offset_pos = offsets_start;

        for _ in 0..num_elements {
            let offset = u32::from_le_bytes([
                data_without_checksum[offset_pos],
                data_without_checksum[offset_pos + 1],
                data_without_checksum[offset_pos + 2],
                data_without_checksum[offset_pos + 3],
            ]);
            offsets.push(offset);
            offset_pos += U32_SIZE;
        }

        Ok(Self {
            data: records_data,
            offsets,
            block_size: BLOCK_SIZE,
        })
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn data_size(&self) -> usize {
        self.data.len()
    }
}
