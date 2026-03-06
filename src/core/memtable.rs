use crate::core::log_record::LogRecord;
use crate::storage::iterator::MemTableIterator;
use std::collections::BTreeMap;

pub struct MemTable {
    pub(crate) data: BTreeMap<String, LogRecord>,
    pub(crate) size_bytes: usize,
    pub(crate) max_size_bytes: usize,
}

impl MemTable {
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            data: BTreeMap::new(),
            size_bytes: 0,
            max_size_bytes,
        }
    }

    pub fn insert(&mut self, record: LogRecord) {
        let record_size = Self::estimate_size(&record);
        if let Some(old_record) = self.data.insert(record.key.clone(), record) {
            self.size_bytes = self
                .size_bytes
                .saturating_sub(Self::estimate_size(&old_record));
        }
        self.size_bytes += record_size;
    }

    pub fn should_flush(&self) -> bool {
        self.size_bytes >= self.max_size_bytes
    }

    pub fn get(&self, key: &str) -> Option<LogRecord> {
        self.data.get(key).cloned()
    }

    /// Returns a StorageIterator over all entries (backward compatible)
    pub fn iter_ordered(&self) -> impl Iterator<Item = (&String, &LogRecord)> {
        self.data.iter()
    }

    /// Returns a MemTableIterator starting from the beginning
    ///
    /// This is the preferred method for using the StorageIterator trait.
    ///
    /// # Example
    /// ```ignore
    /// let mut iter = memtable.iter();
    /// while iter.is_valid() {
    ///     println!("{}={:?}", String::from_utf8_lossy(iter.key()), iter.value());
    ///     iter.next();
    /// }
    /// ```
    pub fn iter(&self) -> MemTableIterator {
        MemTableIterator::new(&self.data)
    }

    /// Returns a MemTableIterator starting from a specific key
    ///
    /// # Arguments
    /// * `start_key` - The key to start iteration from (inclusive)
    ///
    /// # Example
    /// ```ignore
    /// let mut iter = memtable.iter_from("key_100");
    /// while iter.is_valid() {
    ///     // Iterate from key_100 onwards
    ///     iter.next();
    /// }
    /// ```
    pub fn iter_from(&self, start_key: &str) -> MemTableIterator {
        MemTableIterator::new_from(&self.data, start_key)
    }

    pub fn clear(&mut self) -> usize {
        let count = self.data.len();
        self.data.clear();
        self.size_bytes = 0;
        count
    }

    fn estimate_size(record: &LogRecord) -> usize {
        record.key.len() + record.value.len() + 32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::iterator::StorageIterator;

    #[test]
    fn test_memtable_iter() {
        let mut memtable = MemTable::new(1024);
        memtable.insert(LogRecord::new("key1".to_string(), b"value1".to_vec()));
        memtable.insert(LogRecord::new("key2".to_string(), b"value2".to_vec()));
        memtable.insert(LogRecord::new("key3".to_string(), b"value3".to_vec()));

        let mut iter = memtable.iter();
        let mut count = 0;

        while iter.is_valid() {
            count += 1;
            iter.next();
        }

        assert_eq!(count, 3);
    }

    #[test]
    fn test_memtable_iter_from() {
        let mut memtable = MemTable::new(1024);
        memtable.insert(LogRecord::new("key1".to_string(), b"value1".to_vec()));
        memtable.insert(LogRecord::new("key2".to_string(), b"value2".to_vec()));
        memtable.insert(LogRecord::new("key3".to_string(), b"value3".to_vec()));

        let mut iter = memtable.iter_from("key2");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key2");

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key3");

        iter.next();
        assert!(!iter.is_valid());
    }
}
