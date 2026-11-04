//! Shared SSTable metadata validation for discovery, restore and point reads.
use super::{
    builder::MetaBlock,
    encryption::{EncryptionConfig, Encryptor},
};
use crate::infra::{
    codec::decode,
    error::{LsmError, Result},
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

// Bound corrupt length prefixes before allocation. Oversize metadata is an
// unsupported input, not proof of corruption: callers must not quarantine it.
pub(crate) const MAX_METADATA_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn read(
    file: &mut File,
    encryption: &EncryptionConfig,
) -> Result<(MetaBlock, Encryptor)> {
    let length = file.metadata()?.len();
    if length < 16 {
        return Err(LsmError::InvalidSstableFormat(
            "file is shorter than header and footer".into(),
        ));
    }
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    let encryptor = match &magic {
        b"LSMSST03" => Encryptor::disabled(),
        b"LSMSST04" if encryption.enabled => Encryptor::new(encryption),
        b"LSMSST04" => return Err(LsmError::EncryptionKeyRequired),
        _ => {
            return Err(LsmError::InvalidSstableFormat(
                "invalid SSTable magic".into(),
            ))
        }
    };
    file.seek(SeekFrom::End(-8))?;
    let mut footer = [0; 8];
    file.read_exact(&mut footer)?;
    let offset = u64::from_le_bytes(footer);
    if offset < 8 || offset >= length - 8 {
        return Err(LsmError::InvalidSstableFormat(
            "metadata offset lies outside the data area".into(),
        ));
    }
    let size = length - 8 - offset;
    if size > (MAX_METADATA_BYTES + MAX_METADATA_BYTES / 255 + 64) as u64 {
        return Err(LsmError::InvalidArgument(
            "SSTable metadata exceeds the 64 MiB supported limit".into(),
        ));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut data = vec![0; size as usize];
    file.read_exact(&mut data)?;
    let compressed = encryptor.decrypt_block(&data)?;
    let prefix: [u8; 4] = compressed
        .get(..4)
        .and_then(|p| p.try_into().ok())
        .ok_or_else(|| LsmError::InvalidSstableFormat("truncated metadata length prefix".into()))?;
    if u32::from_le_bytes(prefix) as usize > MAX_METADATA_BYTES {
        return Err(LsmError::InvalidArgument(
            "SSTable metadata exceeds the 64 MiB supported limit".into(),
        ));
    }
    let bytes = lz4_flex::decompress_size_prepended(&compressed)
        .map_err(|e| LsmError::DecompressionFailed(format!("metadata: {e}")))?;
    let meta: MetaBlock = decode(&bytes)?;
    if meta.blocks.is_empty()
        || meta.record_count == 0
        || meta.min_key > meta.max_key
        || meta.blocks[0].first_key != meta.min_key
    {
        return Err(LsmError::InvalidSstableFormat(
            "invalid record range or empty block index".into(),
        ));
    }
    let mut previous_end = 8;
    let mut previous_key: Option<&[u8]> = None;
    for block in &meta.blocks {
        if block.size < 4
            || block.offset < previous_end
            || block
                .offset
                .checked_add(u64::from(block.size))
                .is_none_or(|end| end > offset)
            || block.first_key > meta.max_key
            || previous_key.is_some_and(|key| key >= block.first_key.as_slice())
        {
            return Err(LsmError::InvalidSstableFormat(
                "invalid SSTable block bounds or index ordering".into(),
            ));
        }
        previous_end = block.offset + u64::from(block.size);
        previous_key = Some(&block.first_key);
    }
    Ok((meta, encryptor))
}

pub(crate) fn bloom(bytes: Vec<u8>) -> Option<bloomfilter::Bloom<[u8]>> {
    match bloomfilter::Bloom::from_bytes(bytes) {
        Ok(filter) => Some(filter),
        Err(error) => {
            tracing::warn!(target: "apexstore::recovery", %error, "SSTable Bloom filter unavailable; reading without the filter");
            None
        }
    }
}
