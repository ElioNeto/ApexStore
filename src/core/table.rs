#[derive(Clone)]
pub struct Table {
    pub data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    pub level: usize,
    pub path: Option<std::path::PathBuf>,
}

impl Table {
    pub fn build(
        data: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
        _options: &crate::core::engine::EngineOptions,
    ) -> Self {
        Self {
            data,
            level: 0,
            path: None,
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
        let data = if path.exists() {
            std::collections::BTreeMap::new()
        } else {
            std::collections::BTreeMap::new()
        };

        Ok(Self {
            data,
            level: 1, // Assume L1 for compacted tables
            path: Some(path.to_path_buf()),
        })
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
