pub struct Table {
    pub data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    pub level: usize,
    pub path: Option<std::path::PathBuf>,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
    /// Cached bloom filter to avoid opening an SstableReader just for might_contain().
    /// Loaded from the SSTable's MetaBlock when a table is created from a file path.
    pub bloom_filter: Option<bloomfilter::Bloom<[u8]>>,
    // TTL / expires_at metadata is preserved via __ttl:{key} entries
    // in the raw data map (see flush_memtable_impl).  These entries
    // are written alongside real data and persist through flushes and
    // restarts so that reads and scans can correctly detect expiry.
    // Compaction operates on Tables and preserves these side entries.
}

impl Clone for Table {
    fn clone(&self) -> Self {
        let bloom_filter = self.bloom_filter.as_ref().and_then(|bf| {
            let bytes = bf.to_bytes();
            bloomfilter::Bloom::<[u8]>::from_bytes(bytes).ok()
        });
        Self {
            data: self.data.clone(),
            level: self.level,
            path: self.path.clone(),
            min_key: self.min_key.clone(),
            max_key: self.max_key.clone(),
            bloom_filter,
        }
    }
}

impl Table {
    /// Build an in-memory Table from key-value data.
    ///
    /// A Bloom filter is created for the table to accelerate negative
    /// lookups (keys that definitely do not exist).  The false-positive
    /// rate is set to ~1 %.
    pub fn build(
        data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
        _options: &crate::core::engine::EngineOptions,
    ) -> Self {
        let (min_key, max_key) =
            if let (Some(first), Some(last)) = (data.first_key_value(), data.last_key_value()) {
                (first.0.clone(), last.0.clone())
            } else {
                (Vec::new(), Vec::new())
            };

        // Build an in-memory Bloom filter so the engine can quickly reject
        // lookups for absent keys without searching the BTreeMap.
        let bloom_filter = if !data.is_empty() {
            let num_items = data.len();
            match bloomfilter::Bloom::<[u8]>::new_for_fp_rate(num_items, 0.01) {
                Ok(mut bf) => {
                    for key in data.keys() {
                        bf.set(key);
                    }
                    Some(bf)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        Self {
            data,
            level: 0,
            path: None,
            min_key,
            max_key,
            bloom_filter,
        }
    }

    /// Create a new table at a specific level
    pub fn with_level(mut self, level: usize) -> Self {
        self.level = level;
        self
    }

    /// Create a table from an SSTable file path.
    ///
    /// `encryption` controls how the meta block is decrypted on read.
    /// Pass [`EncryptionConfig::default()`] (or `None`) when encryption
    /// is not needed.
    pub fn from_sstable_path(
        path: &std::path::Path,
        encryption: Option<&crate::storage::encryption::EncryptionConfig>,
    ) -> crate::infra::error::Result<Self> {
        // Read the SSTable and extract data
        // For now, we'll create an empty table - in production this would read the SSTable
        let data = std::collections::BTreeMap::new();

        // Extract metadata from the SSTable's MetaBlock
        let (min_key, max_key, bloom_filter) = if path.exists() {
            let default_enc = crate::storage::encryption::EncryptionConfig::default();
            let enc = encryption.unwrap_or(&default_enc);
            match Self::read_meta_block(path, enc) {
                Ok(meta) => {
                    let bf = bloomfilter::Bloom::<[u8]>::from_bytes(meta.bloom_filter_data)
                        .map_err(|e| {
                            crate::infra::error::LsmError::CompactionFailed(format!(
                                "Bloom filter deserialization failed: {}",
                                e
                            ))
                        })?;
                    (meta.min_key, meta.max_key, Some(bf))
                }
                Err(_) => (Vec::new(), Vec::new(), None),
            }
        } else {
            (Vec::new(), Vec::new(), None)
        };

        Ok(Self {
            data,
            level: 1, // Assume L1 for compacted tables
            path: Some(path.to_path_buf()),
            min_key,
            max_key,
            bloom_filter,
        })
    }

    /// Read the MetaBlock from an SSTable file, decrypting if `encryption` is enabled.
    fn read_meta_block(
        path: &std::path::Path,
        encryption: &crate::storage::encryption::EncryptionConfig,
    ) -> crate::infra::error::Result<crate::storage::builder::MetaBlock> {
        use crate::infra::codec::decode;
        use crate::storage::builder::MetaBlock;
        use crate::storage::encryption::Encryptor;
        use lz4_flex::decompress_size_prepended;
        use std::fs::File;
        use std::io::{Read, Seek, SeekFrom};

        const SST_MAGIC_V2: &[u8; 8] = b"LSMSST03";
        const SST_MAGIC_V2_ENCRYPTED: &[u8; 8] = b"LSMSST04";
        const FOOTER_SIZE: u64 = 8;

        let mut file = File::open(path)?;

        // Verify magic number and detect encryption
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;

        let encryptor = Encryptor::new(encryption);

        if &magic != SST_MAGIC_V2 && &magic != SST_MAGIC_V2_ENCRYPTED {
            return Err(crate::infra::error::LsmError::InvalidSstableFormat(
                format!(
                    "Invalid magic number: expected {:?} or {:?}, found {:?}",
                    SST_MAGIC_V2, SST_MAGIC_V2_ENCRYPTED, magic
                ),
            ));
        }

        // If the file is encrypted but no key was provided, fail.
        if &magic == SST_MAGIC_V2_ENCRYPTED && !encryptor.is_enabled() {
            return Err(crate::infra::error::LsmError::InvalidSstableFormat(
                "SSTable is encrypted but no encryption key was provided".to_string(),
            ));
        }

        // Read footer to get metadata offset
        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let mut footer_bytes = [0u8; 8];
        file.read_exact(&mut footer_bytes)?;
        let meta_offset = u64::from_le_bytes(footer_bytes);

        // Read (possibly encrypted) compressed metadata
        file.seek(SeekFrom::Start(meta_offset))?;
        let file_len = file.metadata()?.len();
        let meta_size = (file_len - meta_offset - FOOTER_SIZE) as usize;
        let mut on_disk_meta = vec![0u8; meta_size];
        file.read_exact(&mut on_disk_meta)?;

        // Decrypt first if encryption is enabled
        let compressed_meta = if encryptor.is_enabled() {
            encryptor.decrypt_block(&on_disk_meta)?
        } else {
            on_disk_meta
        };

        // Decompress metadata
        let decompressed = decompress_size_prepended(&compressed_meta).map_err(|e| {
            crate::infra::error::LsmError::DecompressionFailed(format!(
                "Metadata decompression failed: {}",
                e
            ))
        })?;

        // Deserialize metadata
        let metadata: MetaBlock = decode(&decompressed)?;

        Ok(metadata)
    }

    pub fn size(&self) -> usize {
        self.data.iter().map(|(k, v)| k.len() + v.len()).sum()
    }

    pub fn iter(&self) -> TableIterator<'_> {
        TableIterator::new(&self.data)
    }
}

pub struct TableIterator<'a> {
    inner: std::collections::btree_map::Iter<'a, Vec<u8>, Vec<u8>>,
    current: Option<(&'a Vec<u8>, &'a Vec<u8>)>,
}

impl<'a> TableIterator<'a> {
    pub fn new(data: &'a std::collections::BTreeMap<Vec<u8>, Vec<u8>>) -> Self {
        let mut inner = data.iter();
        let current = inner.next();
        Self { inner, current }
    }
}

impl<'a> crate::core::iterators::StorageIterator for TableIterator<'a> {
    type KeyType = Vec<u8>;

    fn next(&mut self) {
        self.current = self.inner.next();
    }
    fn key(&self) -> Self::KeyType {
        match self.current {
            Some((k, _)) => k.clone(),
            None => Vec::new(), // Caller should check is_valid() first
        }
    }
    fn value(&self) -> &[u8] {
        match self.current {
            Some((_, v)) => v.as_slice(),
            None => &[], // Caller should check is_valid() first
        }
    }
    fn is_valid(&self) -> bool {
        self.current.is_some()
    }
    fn seek(&mut self, _key: &[u8]) {
        // Not strictly required for now, but good to have
        while self.is_valid() && self.key().as_slice() < _key {
            self.next();
        }
    }
}
