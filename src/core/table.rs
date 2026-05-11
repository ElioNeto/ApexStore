pub struct Table {
    pub data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    pub level: usize,
    pub path: Option<std::path::PathBuf>,
    pub min_key: Vec<u8>,
    pub max_key: Vec<u8>,
    /// Cached bloom filter to avoid opening an SstableReader just for might_contain().
    /// Loaded from the SSTable's MetaBlock when a table is created from a file path.
    pub bloom_filter: Option<bloomfilter::Bloom<[u8]>>,
}

impl Clone for Table {
    fn clone(&self) -> Self {
        let bloom_filter = self.bloom_filter.as_ref().map(|bf| {
            let bytes = bf.to_bytes();
            bloomfilter::Bloom::<[u8]>::from_bytes(bytes)
                .expect("Bloom filter serialization round-trip should not fail")
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
    pub fn build(
        data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
        _options: &crate::core::engine::EngineOptions,
    ) -> Self {
        let (min_key, max_key) = if let (Some(first), Some(last)) =
            (data.first_key_value(), data.last_key_value())
        {
            (first.0.clone(), last.0.clone())
        } else {
            (Vec::new(), Vec::new())
        };
        Self {
            data,
            level: 0,
            path: None,
            min_key,
            max_key,
            bloom_filter: None,
        }
    }

    /// Create a new table at a specific level
    pub fn with_level(mut self, level: usize) -> Self {
        self.level = level;
        self
    }

    /// Create a table from an SSTable file path
    pub fn from_sstable_path(path: &std::path::Path) -> crate::infra::error::Result<Self> {
        // Read the SSTable and extract data
        // For now, we'll create an empty table - in production this would read the SSTable
        let data = std::collections::BTreeMap::new();

        // Extract metadata from the SSTable's MetaBlock
        let (min_key, max_key, bloom_filter) = if path.exists() {
            match Self::read_meta_block(path) {
                Ok(meta) => {
                    let bf = bloomfilter::Bloom::<[u8]>::from_bytes(meta.bloom_filter_data)
                        .map_err(|e| crate::infra::error::LsmError::CompactionFailed(
                            format!("Bloom filter deserialization failed: {}", e)
                        ))?;
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

    /// Read the MetaBlock from an SSTable file
    fn read_meta_block(path: &std::path::Path) -> crate::infra::error::Result<crate::storage::builder::MetaBlock> {
        use crate::infra::codec::decode;
        use crate::storage::builder::MetaBlock;
        use lz4_flex::decompress_size_prepended;
        use std::fs::File;
        use std::io::{Read, Seek, SeekFrom};

        const SST_MAGIC_V2: &[u8; 8] = b"LSMSST03";
        const FOOTER_SIZE: u64 = 8;

        let mut file = File::open(path)?;

        // Verify magic number
        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != SST_MAGIC_V2 {
            return Err(crate::infra::error::LsmError::InvalidSstableFormat(
                format!("Invalid magic number: expected {:?}, found {:?}", SST_MAGIC_V2, magic)
            ));
        }

        // Read footer to get metadata offset
        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let mut footer_bytes = [0u8; 8];
        file.read_exact(&mut footer_bytes)?;
        let meta_offset = u64::from_le_bytes(footer_bytes);

        // Read compressed metadata
        file.seek(SeekFrom::Start(meta_offset))?;
        let file_len = file.metadata()?.len();
        let meta_size = (file_len - meta_offset - FOOTER_SIZE) as usize;
        let mut compressed_meta = vec![0u8; meta_size];
        file.read_exact(&mut compressed_meta)?;

        // Decompress metadata
        let decompressed = decompress_size_prepended(&compressed_meta)
            .map_err(|e| crate::infra::error::LsmError::DecompressionFailed(
                format!("Metadata decompression failed: {}", e)
            ))?;

        // Deserialize metadata
        let metadata: MetaBlock = decode(&decompressed)?;

        Ok(metadata)
    }

    pub fn size(&self) -> usize {
        self.data
            .iter()
            .map(|(k, v)| k.len() + v.len())
            .sum()
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
    type KeyType = crate::core::key::KeySlice<'a>;

    fn next(&mut self) {
        self.current = self.inner.next();
    }
    fn key(&self) -> Self::KeyType {
        match self.current {
            Some((k, _)) => crate::core::key::KeySlice::new(k.as_slice()),
            None => panic!("current must be Some when key() is called"),
        }
    }
    fn value(&self) -> &[u8] {
        match self.current {
            Some((_, v)) => v.as_slice(),
            None => panic!("current must be Some when value() is called"),
        }
    }
    fn is_valid(&self) -> bool {
        self.current.is_some()
    }
    fn seek(&mut self, _key: &[u8]) {
        // Not strictly required for now, but good to have
        while self.is_valid() && self.key().as_ref() < _key {
            self.next();
        }
    }
}
