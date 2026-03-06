//! Storage Iterator Abstraction
//!
//! This module provides a unified iterator interface (`StorageIterator`) that abstracts
//! iteration over different storage layers (MemTable, SSTable, etc.).
//!
//! The trait enables:
//! - Range queries across storage layers
//! - Merge operations during compaction
//! - Prefix scans and filtered iterations

use crate::core::log_record::LogRecord;
use std::collections::btree_map;

/// Unified iterator interface for storage layers
///
/// This trait provides a common abstraction for iterating over key-value pairs
/// stored in different layers of the LSM tree (MemTable, SSTables).
///
/// # Example
///
/// ```ignore
/// let mut iter = memtable.iter();
/// iter.seek(b"key_100");
/// 
/// while iter.is_valid() {
///     let key = iter.key();
///     let value = iter.value();
///     println!("{}={:?}", String::from_utf8_lossy(key), value);
///     iter.next();
/// }
/// ```
pub trait StorageIterator {
    /// Returns the current key as a byte slice
    ///
    /// # Panics
    /// May panic if called when `is_valid()` returns `false`
    fn key(&self) -> &[u8];

    /// Returns the current value (LogRecord)
    ///
    /// # Panics
    /// May panic if called when `is_valid()` returns `false`
    fn value(&self) -> &LogRecord;

    /// Returns `true` if the iterator is pointing to valid data
    ///
    /// Must be checked before calling `key()` or `value()`
    fn is_valid(&self) -> bool;

    /// Advances the iterator to the next position
    ///
    /// After calling `next()`, you must check `is_valid()` again
    fn next(&mut self);

    /// Positions the iterator at the first key >= `key`
    ///
    /// If no such key exists, the iterator becomes invalid.
    ///
    /// # Arguments
    /// * `key` - The target key to seek to
    fn seek(&mut self, key: &[u8]);
}

/// Iterator over MemTable entries
///
/// Wraps a `BTreeMap::Range` iterator to provide the `StorageIterator` interface.
/// Keys are automatically sorted by the BTreeMap.
pub struct MemTableIterator<'a> {
    inner: btree_map::Range<'a, String, LogRecord>,
    current: Option<(&'a String, &'a LogRecord)>,
}

impl<'a> MemTableIterator<'a> {
    /// Creates a new iterator starting from the beginning of the MemTable
    ///
    /// # Arguments
    /// * `data` - Reference to the BTreeMap backing the MemTable
    pub fn new(data: &'a btree_map::BTreeMap<String, LogRecord>) -> Self {
        let mut inner = data.range::<String, _>(..); // Full range
        let current = inner.next();
        Self { inner, current }
    }

    /// Creates a new iterator starting from a specific key
    ///
    /// # Arguments
    /// * `data` - Reference to the BTreeMap backing the MemTable
    /// * `start_key` - The key to start iteration from (inclusive)
    pub fn new_from(data: &'a btree_map::BTreeMap<String, LogRecord>, start_key: &str) -> Self {
        let mut inner = data.range::<String, _>(start_key.to_string()..); // Range from start_key to end
        let current = inner.next();
        Self { inner, current }
    }
}

impl<'a> StorageIterator for MemTableIterator<'a> {
    fn key(&self) -> &[u8] {
        self.current
            .expect("key() called on invalid iterator")
            .0
            .as_bytes()
    }

    fn value(&self) -> &LogRecord {
        self.current
            .expect("value() called on invalid iterator")
            .1
    }

    fn is_valid(&self) -> bool {
        self.current.is_some()
    }

    fn next(&mut self) {
        self.current = self.inner.next();
    }

    fn seek(&mut self, key: &[u8]) {
        // Convert bytes to String for comparison
        let key_str = String::from_utf8_lossy(key);
        
        // We need to restart the range from the seek position
        // Since we can't modify the inner range in place, we'll iterate until we find it
        while let Some((current_key, _)) = self.current {
            if current_key.as_bytes() >= key {
                // Found a key >= seek target
                return;
            }
            self.current = self.inner.next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn create_test_record(key: &str, value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_string(), value.to_vec())
    }

    fn create_test_memtable() -> BTreeMap<String, LogRecord> {
        let mut map = BTreeMap::new();
        map.insert(
            "key_001".to_string(),
            create_test_record("key_001", b"value_001"),
        );
        map.insert(
            "key_010".to_string(),
            create_test_record("key_010", b"value_010"),
        );
        map.insert(
            "key_020".to_string(),
            create_test_record("key_020", b"value_020"),
        );
        map.insert(
            "key_030".to_string(),
            create_test_record("key_030", b"value_030"),
        );
        map.insert(
            "key_100".to_string(),
            create_test_record("key_100", b"value_100"),
        );
        map
    }

    #[test]
    fn test_iterator_basic() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // First key
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_001");
        assert_eq!(iter.value().value, b"value_001");

        // Second key
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_010");
        assert_eq!(iter.value().value, b"value_010");

        // Third key
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_020");
    }

    #[test]
    fn test_iterator_full_scan() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        let mut count = 0;
        let expected_keys = ["key_001", "key_010", "key_020", "key_030", "key_100"];

        while iter.is_valid() {
            let key = String::from_utf8(iter.key().to_vec()).unwrap();
            assert_eq!(key, expected_keys[count]);
            count += 1;
            iter.next();
        }

        assert_eq!(count, 5, "Should iterate over all 5 keys");
    }

    #[test]
    fn test_iterator_seek_exact() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek to exact key
        iter.seek(b"key_020");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_020");
        assert_eq!(iter.value().value, b"value_020");

        // Continue iterating
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_030");
    }

    #[test]
    fn test_iterator_seek_between() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek to key between existing keys (should find next key)
        iter.seek(b"key_015");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_020"); // Next key after key_015
    }

    #[test]
    fn test_iterator_seek_before_first() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek before first key
        iter.seek(b"key_000");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_001"); // First key
    }

    #[test]
    fn test_iterator_seek_after_last() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek after last key
        iter.seek(b"key_999");
        assert!(!iter.is_valid()); // No keys >= key_999
    }

    #[test]
    fn test_iterator_seek_last_key() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek to last key
        iter.seek(b"key_100");
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_100");

        // Next should be invalid
        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_empty_memtable() {
        let map = BTreeMap::new();
        let mut iter = MemTableIterator::new(&map);

        // Should be invalid from the start
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_single_entry() {
        let mut map = BTreeMap::new();
        map.insert(
            "only_key".to_string(),
            create_test_record("only_key", b"only_value"),
        );

        let mut iter = MemTableIterator::new(&map);

        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"only_key");

        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_new_from() {
        let map = create_test_memtable();
        
        // Start from key_020
        let mut iter = MemTableIterator::new_from(&map, "key_020");
        
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_020");
        
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_030");
        
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_100");
        
        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_deleted_records() {
        let mut map = BTreeMap::new();
        map.insert(
            "key_001".to_string(),
            create_test_record("key_001", b"value_001"),
        );
        map.insert(
            "key_002".to_string(),
            LogRecord::tombstone("key_002".to_string()),
        );
        map.insert(
            "key_003".to_string(),
            create_test_record("key_003", b"value_003"),
        );

        let mut iter = MemTableIterator::new(&map);

        // Should iterate over all entries, including tombstones
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_001");
        assert!(!iter.value().is_deleted);

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_002");
        assert!(iter.value().is_deleted); // Tombstone

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key(), b"key_003");
        assert!(!iter.value().is_deleted);
    }
}
