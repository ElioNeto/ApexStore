//! Block-level key prefix compression for SSTable V2 format.
//!
//! # Overview
//!
//! In an LSM-tree, keys within a single SSTable block are sorted and often share
//! long common prefixes (e.g. `user:alice:`, `user:bob:`, `user:carol:` …).  This
//! module compresses such keys by storing only the **shared prefix length** and
//! the **suffix** for each key relative to its predecessor.
//!
//! # Format
//!
//! Encoded output is a sequence of entries — one per key — each with:
//!
//! | Field              | Type   | Description                                  |
//! |--------------------|--------|----------------------------------------------|
//! | `shared_prefix_len`| u8     | Number of bytes shared with previous key     |
//! | `suffix_len`       | u16    | Length of the suffix (remaining key bytes)   |
//! | `suffix`           | bytes  | The suffix itself (key[shared_prefix_len..]) |
//!
//! For the **first** key, `shared_prefix_len` is 0 and `suffix` is the full key.
//!
//! # Usage
//!
//! ```ignore
//! use apexstore::storage::prefix_compression::PrefixCompressor;
//!
//! let keys = vec![b"user:alice:age".to_vec(), b"user:bob:age".to_vec()];
//! let compressed = PrefixCompressor::encode_keys(&keys);
//! let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]);
//! assert_eq!(keys, decoded);
//! ```

use crate::infra::error::{LsmError, Result};

/// Maximum shared prefix length supported by the u8 encoding (255 bytes).
/// Per-key suffix length is stored as u16, allowing suffixes up to 65535 bytes.
const MAX_SHARED_PREFIX: usize = u8::MAX as usize;

/// Utility for encoding and decoding sorted keys using shared-prefix compression.
pub struct PrefixCompressor;

impl PrefixCompressor {
    /// Encode a sorted sequence of keys into a compact byte representation.
    ///
    /// Each key is encoded relative to its predecessor:
    /// - `shared_prefix_len` (u8) — how many initial bytes are shared
    /// - `suffix_len` (u16, LE) — length of the non-shared suffix
    /// - `suffix` — the remaining key bytes
    ///
    /// The first key always has `shared_prefix_len = 0` (full key stored as suffix).
    ///
    /// # Panics
    ///
    /// Panics if any two consecutive keys share more than 255 prefix bytes.
    pub fn encode_keys(keys: &[Vec<u8>]) -> Result<Vec<u8>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let mut output = Vec::new();
        let mut prev_key: &[u8] = &[];

        for key in keys {
            let shared = Self::shared_prefix_len(prev_key, key);
            if shared > MAX_SHARED_PREFIX {
                return Err(LsmError::InvalidArgument(format!(
                    "shared prefix length {} exceeds maximum {}",
                    shared, MAX_SHARED_PREFIX,
                )));
            }

            let suffix = &key[shared..];
            let suffix_len = suffix.len();

            output.push(shared as u8);
            output.extend_from_slice(&(suffix_len as u16).to_le_bytes());
            output.extend_from_slice(suffix);

            prev_key = key;
        }

        Ok(output)
    }

    /// Decode a prefix-compressed key sequence back into full keys.
    ///
    /// The `data` must be the output of `encode_keys` for the **full** key list
    /// (including the first key).  `first_key` is used as the base for reconstructing
    /// the first key from the encoded data (which stores the first key with
    /// `shared_prefix_len = 0`).
    ///
    /// Returns a `Vec` containing all reconstructed keys.
    ///
    /// # Panics
    ///
    /// Panics if `data` is malformed (truncated, invalid lengths, etc.).
    pub fn decode_keys(data: &[u8], first_key: &[u8]) -> Result<Vec<Vec<u8>>> {
        if data.is_empty() {
            // When there are no encoded keys, just the first_key is the only key.
            // This is the case when we have a block with a single entry.
            return Ok(Vec::new());
        }

        let mut keys: Vec<Vec<u8>> = Vec::new();
        let mut pos = 0;
        let mut prev_key: Vec<u8> = first_key.to_vec();

        while pos < data.len() {
            let shared = data[pos] as usize;
            pos += 1;

            if pos + 2 > data.len() {
                return Err(LsmError::CorruptedData(
                    "Truncated prefix compression data: cannot read suffix_len".to_string(),
                ));
            }
            let suffix_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
            pos += 2;

            if pos + suffix_len > data.len() {
                return Err(LsmError::CorruptedData(
                    "Truncated prefix compression data: suffix extends past end".to_string(),
                ));
            }
            let suffix = &data[pos..pos + suffix_len];
            pos += suffix_len;

            // Reconstruct full key: prev_key[..shared] + suffix
            let mut full_key = Vec::with_capacity(shared + suffix_len);
            full_key.extend_from_slice(&prev_key[..shared]);
            full_key.extend_from_slice(suffix);

            keys.push(full_key);
            prev_key = keys.last().expect("just pushed").clone();
        }

        Ok(keys)
    }

    /// Compress the keys of a block's entries in-place (builds new data + offsets).
    ///
    /// Given the raw block data (with full keys) and the entry offsets, produces
    /// a new data vector where keys are prefix-compressed, and a matching offset
    /// vector pointing into the new data.
    ///
    /// The input `data` must contain entries in the format:
    /// `[key_len(u16)][key_bytes][val_len(u16)][value_bytes]`
    ///
    /// The output format for entry 0 is unchanged (full key).
    /// For entries 1..N, keys are stored as:
    /// `[shared_prefix_len(u8)][suffix_len(u16)][suffix]`
    /// Values are stored as-is: `[val_len(u16)][value_bytes]`
    pub fn compress_block_data(data: &[u8], offsets: &[u32]) -> Result<(Vec<u8>, Vec<u32>)> {
        if offsets.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let mut new_data = Vec::new();
        let mut new_offsets = Vec::with_capacity(offsets.len());
        let mut prev_key: &[u8] = &[];

        for &offset in offsets {
            let offset = offset as usize;
            new_offsets.push(new_data.len() as u32);

            // Read key from original data
            let key_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
            let key = &data[offset + 2..offset + 2 + key_len];

            // Read value
            let val_offset = offset + 2 + key_len;
            let val_len = u16::from_le_bytes([data[val_offset], data[val_offset + 1]]) as usize;
            let value = &data[val_offset + 2..val_offset + 2 + val_len];

            if prev_key.is_empty() {
                // First entry: store full key (standard format)
                new_data.extend_from_slice(&(key_len as u16).to_le_bytes());
                new_data.extend_from_slice(key);
            } else {
                // Subsequent entries: prefix-compressed key
                let shared = Self::shared_prefix_len(prev_key, key);
                if shared > MAX_SHARED_PREFIX {
                    return Err(LsmError::InvalidArgument(format!(
                        "shared prefix length {} exceeds maximum {}",
                        shared, MAX_SHARED_PREFIX,
                    )));
                }
                let suffix = &key[shared..];
                new_data.push(shared as u8);
                new_data.extend_from_slice(&(suffix.len() as u16).to_le_bytes());
                new_data.extend_from_slice(suffix);
            }

            // Write value (same format as before)
            new_data.extend_from_slice(&(val_len as u16).to_le_bytes());
            new_data.extend_from_slice(value);

            prev_key = key;
        }

        Ok((new_data, new_offsets))
    }

    /// Decompress prefix-compressed block data back to the standard format.
    ///
    /// Takes block data where keys (after the first) are prefix-compressed,
    /// and reconstructs the original full-key format with correct offsets.
    ///
    /// Input format per entry:
    /// - Entry 0: `[key_len(u16)][full_key][val_len(u16)][value]`
    /// - Entry i (i>0): `[shared_prefix_len(u8)][suffix_len(u16)][suffix][val_len(u16)][value]`
    pub fn decompress_block_data(data: &[u8], offsets: &[u32]) -> Result<(Vec<u8>, Vec<u32>)> {
        if offsets.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let mut new_data = Vec::new();
        let mut new_offsets = Vec::with_capacity(offsets.len());
        let mut prev_key: Vec<u8> = Vec::new();
        let mut is_first = true;

        for &offset in offsets {
            let offset = offset as usize;
            new_offsets.push(new_data.len() as u32);

            if is_first {
                // First entry: standard format [key_len(u16)][key][val_len(u16)][value]
                if offset + 2 > data.len() {
                    return Err(crate::infra::error::LsmError::CorruptedData(
                        "Prefix-compressed block: truncated first entry (key_len)".to_string(),
                    ));
                }
                let key_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
                if offset + 2 + key_len + 2 > data.len() {
                    return Err(crate::infra::error::LsmError::CorruptedData(
                        "Prefix-compressed block: truncated first entry (value)".to_string(),
                    ));
                }
                let key = &data[offset + 2..offset + 2 + key_len];
                prev_key = key.to_vec();

                let val_offset = offset + 2 + key_len;
                let val_len = u16::from_le_bytes([data[val_offset], data[val_offset + 1]]) as usize;
                let value = &data[val_offset + 2..val_offset + 2 + val_len];

                // Write full key + value (standard format)
                new_data.extend_from_slice(&(key_len as u16).to_le_bytes());
                new_data.extend_from_slice(key);
                new_data.extend_from_slice(&(val_len as u16).to_le_bytes());
                new_data.extend_from_slice(value);

                is_first = false;
            } else {
                // Subsequent entries: [shared(u8)][suffix_len(u16)][suffix][val_len(u16)][value]
                if offset + 1 > data.len() {
                    return Err(crate::infra::error::LsmError::CorruptedData(
                        "Prefix-compressed block: truncated entry (shared)".to_string(),
                    ));
                }
                let shared = data[offset] as usize;
                if offset + 1 + 2 > data.len() {
                    return Err(crate::infra::error::LsmError::CorruptedData(
                        "Prefix-compressed block: truncated entry (suffix_len)".to_string(),
                    ));
                }
                let suffix_len = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as usize;
                let suffix_start = offset + 1 + 2;
                if suffix_start + suffix_len + 2 > data.len() {
                    return Err(crate::infra::error::LsmError::CorruptedData(
                        "Prefix-compressed block: truncated entry (value)".to_string(),
                    ));
                }
                let suffix = &data[suffix_start..suffix_start + suffix_len];

                // Reconstruct full key
                let full_key: Vec<u8> = prev_key[..shared]
                    .iter()
                    .chain(suffix.iter())
                    .copied()
                    .collect();

                let val_offset = suffix_start + suffix_len;
                let val_len = u16::from_le_bytes([data[val_offset], data[val_offset + 1]]) as usize;
                let value = &data[val_offset + 2..val_offset + 2 + val_len];

                // Write full key + value (standard format)
                let key_len = full_key.len();
                new_data.extend_from_slice(&(key_len as u16).to_le_bytes());
                new_data.extend_from_slice(&full_key);
                new_data.extend_from_slice(&(val_len as u16).to_le_bytes());
                new_data.extend_from_slice(value);

                prev_key = full_key;
            }
        }

        Ok((new_data, new_offsets))
    }

    /// Compute the length of the common prefix between two byte slices.
    fn shared_prefix_len(a: &[u8], b: &[u8]) -> usize {
        a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_empty() {
        let keys: Vec<Vec<u8>> = vec![];
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        assert!(compressed.is_empty());

        let decoded = PrefixCompressor::decode_keys(&compressed, b"first_key").unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_encode_decode_single_key() {
        let keys = vec![b"hello".to_vec()];
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]).unwrap();
        assert_eq!(keys, decoded);
    }

    #[test]
    fn test_encode_decode_multiple_keys() {
        let keys = vec![
            b"user:alice:age".to_vec(),
            b"user:bob:age".to_vec(),
            b"user:carol:age".to_vec(),
            b"user:dave:score".to_vec(),
        ];
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]).unwrap();
        assert_eq!(keys, decoded);
    }

    #[test]
    fn test_encode_decode_no_shared_prefix() {
        let keys = vec![b"aaaa".to_vec(), b"bbbb".to_vec(), b"cccc".to_vec()];
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]).unwrap();
        assert_eq!(keys, decoded);
    }

    #[test]
    fn test_encode_decode_identical_keys() {
        let keys = vec![
            b"samekey".to_vec(),
            b"samekey".to_vec(),
            b"samekey".to_vec(),
        ];
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]).unwrap();
        assert_eq!(keys, decoded);
    }

    #[test]
    fn test_encode_decode_long_prefix() {
        let prefix = "A".repeat(200);
        let mut keys: Vec<Vec<u8>> = Vec::new();
        for i in 0..5u8 {
            let mut k = prefix.as_bytes().to_vec();
            k.push(b'a' + i);
            keys.push(k);
        }
        let compressed = PrefixCompressor::encode_keys(&keys).unwrap();
        let decoded = PrefixCompressor::decode_keys(&compressed, &keys[0]).unwrap();
        assert_eq!(keys, decoded);
    }

    #[test]
    fn test_compress_block_data_basic() {
        // Build block data with 3 entries: [key_len(u16)][key][val_len(u16)][value]
        let mut data = Vec::new();
        let mut offsets = Vec::new();

        // Entry 0: key="aaa", value="v1"
        offsets.push(data.len() as u32);
        data.extend_from_slice(&(3u16).to_le_bytes()); // key_len
        data.extend_from_slice(b"aaa");
        data.extend_from_slice(&(2u16).to_le_bytes()); // val_len
        data.extend_from_slice(b"v1");

        // Entry 1: key="aab", value="v2"
        offsets.push(data.len() as u32);
        data.extend_from_slice(&(3u16).to_le_bytes()); // key_len
        data.extend_from_slice(b"aab");
        data.extend_from_slice(&(2u16).to_le_bytes()); // val_len
        data.extend_from_slice(b"v2");

        // Entry 2: key="aac", value="v3"
        offsets.push(data.len() as u32);
        data.extend_from_slice(&(3u16).to_le_bytes()); // key_len
        data.extend_from_slice(b"aac");
        data.extend_from_slice(&(2u16).to_le_bytes()); // val_len
        data.extend_from_slice(b"v3");

        let (compressed_data, new_offsets) =
            PrefixCompressor::compress_block_data(&data, &offsets).unwrap();

        // First entry should be full key "aaa"
        let key0_len = u16::from_le_bytes([compressed_data[0], compressed_data[1]]) as usize;
        assert_eq!(key0_len, 3);
        assert_eq!(&compressed_data[2..5], b"aaa");
        // Value: v1
        let v0_offset = 2 + 3;
        let v0_len =
            u16::from_le_bytes([compressed_data[v0_offset], compressed_data[v0_offset + 1]])
                as usize;
        assert_eq!(v0_len, 2);
        assert_eq!(&compressed_data[v0_offset + 2..v0_offset + 2 + 2], b"v1");

        // Second entry: compressed
        let e1_start = new_offsets[1] as usize;
        let shared1 = compressed_data[e1_start];
        assert_eq!(shared1, 2); // shared "aa"
        let suffix_len1 =
            u16::from_le_bytes([compressed_data[e1_start + 1], compressed_data[e1_start + 2]])
                as usize;
        assert_eq!(suffix_len1, 1);
        assert_eq!(compressed_data[e1_start + 3], b'b');

        // Third entry: compressed
        let e2_start = new_offsets[2] as usize;
        let shared2 = compressed_data[e2_start];
        assert_eq!(shared2, 2); // shared "aa"
        let suffix_len2 =
            u16::from_le_bytes([compressed_data[e2_start + 1], compressed_data[e2_start + 2]])
                as usize;
        assert_eq!(suffix_len2, 1);
        assert_eq!(compressed_data[e2_start + 3], b'c');
    }

    #[test]
    fn test_compress_decompress_roundtrip_block() {
        // Build block data with entries
        let mut data = Vec::new();
        let mut offsets = Vec::new();

        let entries: Vec<(&[u8], &[u8])> = vec![
            (b"user:alice:name", b"Alice"),
            (b"user:bob:name", b"Bob"),
            (b"user:carol:name", b"Carol"),
            (b"user:dave:age", b"42"),
        ];

        for (key, value) in &entries {
            offsets.push(data.len() as u32);
            data.extend_from_slice(&(key.len() as u16).to_le_bytes());
            data.extend_from_slice(key);
            data.extend_from_slice(&(value.len() as u16).to_le_bytes());
            data.extend_from_slice(value);
        }

        let (compressed_data, compressed_offsets) =
            PrefixCompressor::compress_block_data(&data, &offsets).unwrap();

        let (decompressed_data, decompressed_offsets) =
            PrefixCompressor::decompress_block_data(&compressed_data, &compressed_offsets).unwrap();

        assert_eq!(data, decompressed_data);
        assert_eq!(offsets, decompressed_offsets);
    }

    #[test]
    fn test_compress_decompress_single_entry() {
        let mut data = Vec::new();
        let offsets = vec![0u32];
        data.extend_from_slice(&(3u16).to_le_bytes());
        data.extend_from_slice(b"abc");
        data.extend_from_slice(&(3u16).to_le_bytes());
        data.extend_from_slice(b"val");

        let (compressed_data, compressed_offsets) =
            PrefixCompressor::compress_block_data(&data, &offsets).unwrap();
        let (decompressed_data, decompressed_offsets) =
            PrefixCompressor::decompress_block_data(&compressed_data, &compressed_offsets).unwrap();

        assert_eq!(data, decompressed_data);
        assert_eq!(offsets, decompressed_offsets);
    }
}
