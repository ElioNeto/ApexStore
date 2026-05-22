use crate::infra::{config::StorageConfig, error::LsmError};
use crate::storage::prefix_compression::PrefixCompressor;
use crc32fast::Hasher;
use std::mem::size_of;

pub const BLOCK_SIZE: usize = 4096;
const U32_SIZE: usize = size_of::<u32>();

/// Flags bit: when set, keys within this block use shared-prefix encoding.
const PREFIX_COMPRESSION_FLAG: u8 = 0b0000_0001;

/// Additional byte inserted between `num_elements` and CRC32 in the encoded format.
const FLAGS_SIZE: usize = 1;

#[derive(Debug, Clone)]
pub struct Block {
    pub(crate) data: Vec<u8>,
    pub(crate) offsets: Vec<u32>,
    block_size: usize,
    /// Bit flags stored in the encoded block format.
    flags: u8,
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
            flags: 0,
        }
    }

    /// Returns `true` if this block was decoded from prefix-compressed data.
    pub fn is_prefix_compressed(&self) -> bool {
        self.flags & PREFIX_COMPRESSION_FLAG != 0
    }

    /// Mark the block as prefix-compressed (called by the builder after compressing keys).
    pub fn set_prefix_compressed(&mut self) {
        self.flags |= PREFIX_COMPRESSION_FLAG;
    }

    /// Compress keys using prefix encoding, modifying `data` and `offsets` in place.
    /// This should be called **before** `encode()` when building an SSTable.
    pub fn compress_keys(&mut self) {
        if self.offsets.is_empty() {
            return;
        }
        let (new_data, new_offsets) =
            PrefixCompressor::compress_block_data(&self.data, &self.offsets);
        self.data = new_data;
        self.offsets = new_offsets;
        self.flags |= PREFIX_COMPRESSION_FLAG;
    }

    fn entry_size(key: &[u8], value: &[u8]) -> usize {
        // KeyLen(2) + Key + ValLen(2) + Value
        2 + key.len() + 2 + value.len()
    }

    fn metadata_size(num_entries: usize) -> usize {
        (num_entries * U32_SIZE) + U32_SIZE + FLAGS_SIZE
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
        let mut encoded = Vec::with_capacity(self.current_size() + FLAGS_SIZE);
        encoded.extend_from_slice(&self.data);

        for &offset in &self.offsets {
            encoded.extend_from_slice(&offset.to_le_bytes());
        }

        let num_elements = self.offsets.len() as u32;
        encoded.extend_from_slice(&num_elements.to_le_bytes());

        // Insert flags byte between num_elements and CRC32
        encoded.push(self.flags);

        // Calculate and append CRC32 checksum (Little Endian)
        let mut hasher = Hasher::new();
        hasher.update(&encoded);
        let checksum = hasher.finalize();
        encoded.extend_from_slice(&checksum.to_le_bytes());

        encoded
    }

    pub fn decode(data: &[u8]) -> std::result::Result<Self, LsmError> {
        if data.len() < 2 * U32_SIZE + FLAGS_SIZE {
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

        // Read flags byte (right before CRC32, after num_elements)
        let flags_pos = data_without_checksum.len() - FLAGS_SIZE;
        let flags = data_without_checksum[flags_pos];

        // num_elements is before the flags byte
        let num_elements_start = flags_pos - U32_SIZE;
        let num_elements = u32::from_le_bytes([
            data_without_checksum[num_elements_start],
            data_without_checksum[num_elements_start + 1],
            data_without_checksum[num_elements_start + 2],
            data_without_checksum[num_elements_start + 3],
        ]) as usize;

        let offsets_start = num_elements_start - (num_elements * U32_SIZE);
        let raw_data = data_without_checksum[..offsets_start].to_vec();

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

        let is_compressed = flags & PREFIX_COMPRESSION_FLAG != 0;

        if is_compressed {
            // Decompress keys: rebuild full keys from prefix-compressed entries
            let (decompressed_data, decompressed_offsets) =
                PrefixCompressor::decompress_block_data(&raw_data, &offsets)?;
            Ok(Self {
                data: decompressed_data,
                offsets: decompressed_offsets,
                block_size: BLOCK_SIZE,
                flags,
            })
        } else {
            Ok(Self {
                data: raw_data,
                offsets,
                block_size: BLOCK_SIZE,
                flags,
            })
        }
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
