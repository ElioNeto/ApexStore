//! Storage Iterator Abstraction
//!
//! This module provides a unified iterator interface (`StorageIterator`) that abstracts
//! iteration over different storage layers (MemTable, SSTable, etc.).
//!
//! The trait enables:
//! - Range queries across storage layers
//! - Merge operations during compaction
//! - Prefix scans and filtered iterations

use crate::core::iterators::StorageIterator;
use crate::core::key::KeySlice;
use crate::core::log_record::LogRecord;
use std::collections::btree_map;

/// Iterator over MemTable entries
///
/// Wraps a `BTreeMap::Range` iterator to provide the `StorageIterator` interface.
/// Keys are automatically sorted by the BTreeMap.
pub struct MemTableIterator<'a> {
    inner: btree_map::Range<'a, Vec<u8>, LogRecord>,
    current: Option<(&'a Vec<u8>, &'a LogRecord)>,
}

impl<'a> MemTableIterator<'a> {
    /// Creates a new iterator starting from the beginning of the MemTable
    ///
    /// # Arguments
    /// * `data` - Reference to the BTreeMap backing the MemTable
    pub fn new(data: &'a btree_map::BTreeMap<Vec<u8>, LogRecord>) -> Self {
        let mut inner = data.range::<Vec<u8>, _>(..); // Full range
        let current = inner.next();
        Self { inner, current }
    }

    /// Creates a new iterator starting from a specific key
    ///
    /// # Arguments
    /// * `data` - Reference to the BTreeMap backing the MemTable
    /// * `start_key` - The key to start iteration from (inclusive)
    pub fn new_from(data: &'a btree_map::BTreeMap<Vec<u8>, LogRecord>, start_key: &[u8]) -> Self {
        let mut inner = data.range::<Vec<u8>, _>(start_key.to_vec()..); // Range from start_key to end
        let current = inner.next();
        Self { inner, current }
    }
}

impl<'a> StorageIterator for MemTableIterator<'a> {
    type KeyType = KeySlice<'a>;

    fn key(&self) -> Self::KeyType {
        KeySlice::new(
            self.current
                .expect("key() called on invalid iterator")
                .0
                .as_slice(),
        )
    }

    fn value(&self) -> &[u8] {
        &self
            .current
            .expect("value() called on invalid iterator")
            .1
            .value
    }

    fn is_valid(&self) -> bool {
        self.current.is_some()
    }

    fn next(&mut self) {
        self.current = self.inner.next();
    }

    fn seek(&mut self, key: &[u8]) {
        // We need to iterate until we find a key >= seek target
        while let Some((current_key, _)) = self.current {
            if current_key.as_slice() >= key {
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

    fn create_test_record(key: &[u8], value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_vec(), value.to_vec())
    }

    fn create_test_memtable() -> BTreeMap<Vec<u8>, LogRecord> {
        let mut map = BTreeMap::new();
        map.insert(
            b"key_001".to_vec(),
            create_test_record(b"key_001", b"value_001"),
        );
        map.insert(
            b"key_010".to_vec(),
            create_test_record(b"key_010", b"value_010"),
        );
        map.insert(
            b"key_020".to_vec(),
            create_test_record(b"key_020", b"value_020"),
        );
        map.insert(
            b"key_030".to_vec(),
            create_test_record(b"key_030", b"value_030"),
        );
        map.insert(
            b"key_100".to_vec(),
            create_test_record(b"key_100", b"value_100"),
        );
        map
    }

    #[test]
    fn test_iterator_basic() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // First key
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_001");
        assert_eq!(iter.value(), b"value_001");

        // Second key
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_010");
        assert_eq!(iter.value(), b"value_010");

        // Third key
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_020");
    }

    #[test]
    fn test_iterator_full_scan() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        let mut count = 0;
        let expected_keys = [b"key_001", b"key_010", b"key_020", b"key_030", b"key_100"];

        while iter.is_valid() {
            assert_eq!(iter.key().as_slice(), expected_keys[count]);
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
        assert_eq!(iter.key().as_slice(), b"key_020");
        assert_eq!(iter.value(), b"value_020");

        // Continue iterating
        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_030");
    }

    #[test]
    fn test_iterator_seek_between() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek to key between existing keys (should find next key)
        iter.seek(b"key_015");
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_020"); // Next key after key_015
    }

    #[test]
    fn test_iterator_seek_before_first() {
        let map = create_test_memtable();
        let mut iter = MemTableIterator::new(&map);

        // Seek before first key
        iter.seek(b"key_000");
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_001"); // First key
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
        assert_eq!(iter.key().as_slice(), b"key_100");

        // Next should be invalid
        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_empty_memtable() {
        let map = BTreeMap::new();
        let iter = MemTableIterator::new(&map);

        // Should be invalid from the start
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_single_entry() {
        let mut map = BTreeMap::new();
        map.insert(
            b"only_key".to_vec(),
            create_test_record(b"only_key", b"only_value"),
        );

        let mut iter = MemTableIterator::new(&map);

        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"only_key");

        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_new_from() {
        let map = create_test_memtable();

        // Start from key_020
        let mut iter = MemTableIterator::new_from(&map, b"key_020");

        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_020");

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_030");

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_100");

        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_deleted_records() {
        let mut map = BTreeMap::new();
        map.insert(
            b"key_001".to_vec(),
            create_test_record(b"key_001", b"value_001"),
        );
        map.insert(
            b"key_002".to_vec(),
            LogRecord::tombstone(b"key_002".to_vec()),
        );
        map.insert(
            b"key_003".to_vec(),
            create_test_record(b"key_003", b"value_003"),
        );

        let mut iter = MemTableIterator::new(&map);

        // Should iterate over all entries, including tombstones
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_001");
        assert!(!iter.value().is_empty()); // Not a tombstone

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_002");
        assert!(iter.value().is_empty()); // Tombstone

        iter.next();
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_003");
        assert!(!iter.value().is_empty()); // Not a tombstone
    }
}
