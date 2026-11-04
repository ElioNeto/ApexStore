/// Outcome of inspecting an existing SSTable. Configuration/authentication
/// failures are returned separately and must never move the original file.
pub enum TableLoad {
    Healthy(Table),
    DegradedBloom(Table),
    Corrupt(crate::infra::error::LsmError),
}

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

    /// Load validated metadata; Bloom failure degrades reads but never hides data.
    /// The on-disk magic determines whether an available key is needed.
    pub fn from_sstable_path(
        path: &std::path::Path,
        encryption: Option<&crate::storage::encryption::EncryptionConfig>,
    ) -> crate::infra::error::Result<Self> {
        let default_enc = crate::storage::encryption::EncryptionConfig::default();
        let mut file = std::fs::File::open(path)?;
        let (meta, _) =
            crate::storage::metadata::read(&mut file, encryption.unwrap_or(&default_enc))?;
        Ok(Self {
            data: std::collections::BTreeMap::new(),
            level: 1,
            path: Some(path.to_path_buf()),
            min_key: meta.min_key,
            max_key: meta.max_key,
            bloom_filter: crate::storage::metadata::bloom(meta.bloom_filter_data),
        })
    }

    /// Classify structural damage separately from key, permission and I/O failures.
    /// Only proven structural damage is eligible for quarantine.
    pub fn load_for_recovery(
        path: &std::path::Path,
        encryption: &crate::storage::encryption::EncryptionConfig,
    ) -> crate::infra::error::Result<TableLoad> {
        use crate::infra::error::LsmError;
        match Self::from_sstable_path(path, Some(encryption)) {
            Ok(table) if table.bloom_filter.is_none() => Ok(TableLoad::DegradedBloom(table)),
            Ok(table) => Ok(TableLoad::Healthy(table)),
            Err(
                error @ (LsmError::InvalidSstableFormat(_)
                | LsmError::CorruptedData(_)
                | LsmError::DecompressionFailed(_)
                | LsmError::Codec(_)),
            ) => Ok(TableLoad::Corrupt(error)),
            Err(LsmError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                Ok(TableLoad::Corrupt(LsmError::Io(error)))
            }
            Err(error) => Err(error),
        }
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
