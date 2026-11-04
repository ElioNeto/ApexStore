use crate::core::log_record::{LogRecord, RangeTombstone};
use crate::storage::iterator::MemTableIterator;
use dashmap::DashMap;

pub struct MemTable {
    pub(crate) data: DashMap<Vec<u8>, LogRecord>,
    pub(crate) size_bytes: usize,
    pub(crate) max_size_bytes: usize,
    /// Active range tombstones that apply to this memtable's data.
    pub(crate) range_tombstones: Vec<RangeTombstone>,
}

impl MemTable {
    /// Create a new MemTable with a maximum size limit.
    ///
    /// When `max_size_bytes` is 0 the table has no effective limit and
    /// `should_flush()` always returns `false`.
    pub fn new(max_size_bytes: usize) -> Self {
        Self {
            data: DashMap::new(),
            size_bytes: 0,
            max_size_bytes,
            range_tombstones: Vec::new(),
        }
    }

    /// Create a new MemTable with no size limit (convenience).
    pub fn new_unlimited() -> Self {
        Self::new(0)
    }

    /// Insert a key-value pair, wrapping it in a `LogRecord`.
    ///
    /// Equivalent to `self.insert(LogRecord::new(key, value))`.
    pub fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.insert(LogRecord::new(key, value));
    }

    /// Insert a tombstone (delete marker) for the given key.
    ///
    /// Equivalent to `self.insert(LogRecord::tombstone(key))`.
    pub fn delete(&mut self, key: Vec<u8>) {
        self.insert(LogRecord::tombstone(key));
    }

    pub fn insert(&mut self, record: LogRecord) {
        let record_size = Self::estimate_size(&record);
        let key = record.key.clone();
        if let Some(old_record) = self.data.insert(key, record) {
            self.size_bytes = self
                .size_bytes
                .saturating_sub(Self::estimate_size(&old_record));
        }
        self.size_bytes = self.size_bytes.saturating_add(record_size);
    }

    pub fn should_flush(&self) -> bool {
        self.size_bytes >= self.max_size_bytes
    }

    pub fn get(&self, key: &[u8]) -> Option<LogRecord> {
        self.data.get(key).map(|r| r.clone())
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
        // Collect a snapshot sorted by key (DashMap does not guarantee order)
        let mut entries: Vec<(Vec<u8>, LogRecord)> = self
            .data
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        MemTableIterator::new(entries)
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
    pub fn iter_from(&self, start_key: &[u8]) -> MemTableIterator {
        // Collect a snapshot filtered by start_key, then sort
        let mut entries: Vec<(Vec<u8>, LogRecord)> = self
            .data
            .iter()
            .filter(|e| e.key().as_slice() >= start_key)
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        MemTableIterator::new(entries)
    }

    /// Add a range tombstone covering [start, end).
    pub fn add_range_tombstone(&mut self, range: RangeTombstone) {
        self.range_tombstones.push(range);
    }

    pub fn iter_ordered(&self) -> impl Iterator<Item = (Vec<u8>, LogRecord)> + '_ {
        let mut entries: Vec<_> = self
            .data
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.into_iter()
    }

    /// Check if a key falls within any active range tombstone.
    ///
    /// Returns `true` if the key is covered by any range tombstone
    /// (i.e. `start_key <= key < end_key`).
    pub fn contains_range_tombstone(&self, key: &[u8]) -> bool {
        self.max_covering_range_tombstone(key).is_some()
    }

    /// Timestamp of the newest range tombstone covering `key`, if any.
    ///
    /// Callers need the timestamp, not just a boolean: whether a range
    /// tombstone hides a point write depends on which came first. Comparing
    /// timestamps is the only way to order them, because a point write and the
    /// tombstone that covers it live in different structures.
    pub fn max_covering_range_tombstone(&self, key: &[u8]) -> Option<u128> {
        self.range_tombstones
            .iter()
            .filter(|rt| rt.start_key.as_slice() <= key && key < rt.end_key.as_slice())
            .map(|rt| rt.timestamp)
            .max()
    }

    pub fn clear(&mut self) -> usize {
        let count = self.data.len();
        self.data.clear();
        self.range_tombstones.clear();
        self.size_bytes = 0;
        count
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn size(&self) -> usize {
        self.size_bytes
    }

    fn estimate_size(record: &LogRecord) -> usize {
        // Base overhead: timestamp(16) + is_deleted(1) + column_family tag(1) +
        //               expires_at tag(1) + expires_at data(16) + misc(16) = ~51
        record.key.len() + record.value.len() + 51
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::iterators::StorageIterator;

    #[test]
    fn test_memtable_iter() {
        let mut memtable = MemTable::new(1024);
        memtable.insert(LogRecord::new(b"key1".to_vec(), b"value1".to_vec()));
        memtable.insert(LogRecord::new(b"key2".to_vec(), b"value2".to_vec()));
        memtable.insert(LogRecord::new(b"key3".to_vec(), b"value3".to_vec()));

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
        memtable.insert(LogRecord::new(b"key1".to_vec(), b"value1".to_vec()));
        memtable.insert(LogRecord::new(b"key2".to_vec(), b"value2".to_vec()));
        memtable.insert(LogRecord::new(b"key3".to_vec(), b"value3".to_vec()));

        let mut iter = memtable.iter_from(b"key2");
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key2");

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key3");

        iter.next();
        assert!(!iter.is_valid());
    }
}
