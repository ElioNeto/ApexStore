use crate::infra::config::StorageConfig;
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
        // Note: Using u16 for key/value length storage within the block data
        // to maintain compactness for individual entries, while allowing
        // the overall block to be larger than 64KB via u32 offsets.
        2 + key.len() + 2 + value.len()
    }

    fn metadata_size(num_entries: usize) -> usize {
        (num_entries * U32_SIZE) + U32_SIZE
    }

    fn current_size(&self) -> usize {
        self.data.len() + Self::metadata_size(self.offsets.len())
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

        encoded
    }

    pub fn decode(data: &[u8]) -> Self {
        if data.len() < U32_SIZE {
            return Self {
                data: Vec::new(),
                offsets: Vec::new(),
                block_size: BLOCK_SIZE,
            };
        }

        let num_elements_start = data.len() - U32_SIZE;
        let num_elements = u32::from_le_bytes([
            data[num_elements_start],
            data[num_elements_start + 1],
            data[num_elements_start + 2],
            data[num_elements_start + 3],
        ]) as usize;

        let offsets_start = data.len() - U32_SIZE - (num_elements * U32_SIZE);
        let records_data = data[..offsets_start].to_vec();

        let mut offsets = Vec::with_capacity(num_elements);
        let mut offset_pos = offsets_start;

        for _ in 0..num_elements {
            let offset = u32::from_le_bytes([
                data[offset_pos],
                data[offset_pos + 1],
                data[offset_pos + 2],
                data[offset_pos + 3],
            ]);
            offsets.push(offset);
            offset_pos += U32_SIZE;
        }

        Self {
            data: records_data,
            offsets,
            block_size: BLOCK_SIZE,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_block_is_empty() {
        let block = Block::new(BLOCK_SIZE);
        assert_eq!(block.len(), 0);
        assert!(block.is_empty());
        assert_eq!(block.data_size(), 0);
    }

    #[test]
    fn test_add_single_entry() {
        let mut block = Block::new(BLOCK_SIZE);
        let key = b"test_key";
        let value = b"test_value";
        let success = block.add(key, value);
        assert!(success, "Should successfully add entry");
        assert_eq!(block.len(), 1);
        assert!(!block.is_empty());
    }

    #[test]
    fn test_add_multiple_entries() {
        let mut block = Block::new(BLOCK_SIZE);
        for i in 0..10 {
            let key = format!("key_{:03}", i);
            let value = format!("value_{:03}", i);
            let success = block.add(key.as_bytes(), value.as_bytes());
            assert!(success, "Should add entry {}", i);
        }
        assert_eq!(block.len(), 10);
    }

    #[test]
    fn test_add_until_full() {
        let mut block = Block::new(256);
        let mut added_count = 0;

        for i in 0..100 {
            let key = format!("k{}", i);
            let value = format!("v{}", i);
            if block.add(key.as_bytes(), value.as_bytes()) {
                added_count += 1;
            } else {
                break;
            }
        }

        assert!(added_count > 0, "Should have added at least one entry");
        assert!(
            added_count < 100,
            "Should not have added all entries (block is full)"
        );

        let result = block.add(b"extra_key", b"extra_value");
        assert!(!result, "Should reject entry when block is full");
    }

    #[test]
    fn test_block_overflow_u16() {
        // Create a block larger than 64KB (u16::MAX is 65535)
        let block_size = 70_000;
        let mut block = Block::new(block_size);

        // Fill with enough data to exceed 64KB
        // Each entry approx 1024 bytes
        let val_size = 1000;
        let large_value = vec![b'x'; val_size];
        let key_base = "key";

        let mut count = 0;
        while block.data_size() < 66000 {
            let key = format!("{}{}", key_base, count);
            if !block.add(key.as_bytes(), &large_value) {
                break;
            }
            count += 1;
        }

        assert!(
            block.data_size() > 65535,
            "Block data size should be > 64KB"
        );

        // Verify integrity
        let encoded = block.encode();
        let decoded = Block::decode(&encoded);

        assert_eq!(decoded.len(), block.len());
        assert_eq!(decoded.offsets.len(), block.offsets.len());

        // Verify last entry is correct
        let last_offset = *decoded.offsets.last().unwrap();
        assert!(last_offset > 65535, "Last offset should exceed u16 limit");
    }

    #[test]
    fn test_overflow_large_entry() {
        let mut block = Block::new(128);
        let large_key = vec![b'x'; 100];
        let large_value = vec![b'y'; 100];
        let result = block.add(&large_key, &large_value);
        assert!(!result, "Should reject oversized entry");
        assert_eq!(block.len(), 0, "Block should remain empty");
    }

    #[test]
    fn test_encode_decode_empty_block() {
        let block = Block::new(BLOCK_SIZE);
        let encoded = block.encode();
        let decoded = Block::decode(&encoded);
        assert_eq!(decoded.len(), 0);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_encode_decode_single_entry() {
        let mut block = Block::new(BLOCK_SIZE);
        block.add(b"key1", b"value1");
        let encoded = block.encode();
        let decoded = Block::decode(&encoded);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded.data_size(), block.data_size());
        assert_eq!(decoded.data, block.data);
        assert_eq!(decoded.offsets, block.offsets);
    }

    #[test]
    fn test_encode_decode_multiple_entries() {
        let mut block = Block::new(BLOCK_SIZE);
        let entries = vec![
            (b"apple" as &[u8], b"red" as &[u8]),
            (b"banana", b"yellow"),
            (b"cherry", b"red"),
            (b"date", b"brown"),
            (b"elderberry", b"purple"),
        ];

        for (key, value) in &entries {
            assert!(block.add(key, value));
        }

        let encoded = block.encode();
        let decoded = Block::decode(&encoded);
        assert_eq!(decoded.len(), entries.len());
        assert_eq!(decoded.data, block.data);
        assert_eq!(decoded.offsets, block.offsets);
    }
}
