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
use crate::core::log_record::LogRecord;

/// Iterator over MemTable entries backed by a sorted snapshot.
///
/// Since `DashMap` iterators are not ordered, we take a sorted snapshot
/// at creation time and iterate over that.
pub struct MemTableIterator {
    entries: Vec<(Vec<u8>, LogRecord)>,
    pos: usize,
}

impl MemTableIterator {
    /// Creates a new iterator from a sorted snapshot of entries.
    ///
    /// # Arguments
    /// * `entries` - A sorted vector of (key, LogRecord) pairs
    pub fn new(entries: Vec<(Vec<u8>, LogRecord)>) -> Self {
        Self { entries, pos: 0 }
    }
}

impl StorageIterator for MemTableIterator {
    type KeyType = Vec<u8>;

    fn key(&self) -> Self::KeyType {
        if self.pos < self.entries.len() {
            self.entries[self.pos].0.clone()
        } else {
            Vec::new()
        }
    }

    fn value(&self) -> &[u8] {
        if self.pos < self.entries.len() {
            &self.entries[self.pos].1.value
        } else {
            &[]
        }
    }

    fn is_valid(&self) -> bool {
        self.pos < self.entries.len()
    }

    fn next(&mut self) {
        self.pos += 1;
    }

    fn seek(&mut self, key: &[u8]) {
        while self.pos < self.entries.len() && self.entries[self.pos].0.as_slice() < key {
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_record(key: &[u8], value: &[u8]) -> LogRecord {
        LogRecord::new(key.to_vec(), value.to_vec())
    }

    fn create_test_entries() -> Vec<(Vec<u8>, LogRecord)> {
        vec![
            (
                b"key_001".to_vec(),
                create_test_record(b"key_001", b"value_001"),
            ),
            (
                b"key_010".to_vec(),
                create_test_record(b"key_010", b"value_010"),
            ),
            (
                b"key_020".to_vec(),
                create_test_record(b"key_020", b"value_020"),
            ),
            (
                b"key_030".to_vec(),
                create_test_record(b"key_030", b"value_030"),
            ),
            (
                b"key_100".to_vec(),
                create_test_record(b"key_100", b"value_100"),
            ),
        ]
    }

    #[test]
    fn test_iterator_basic() {
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

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
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

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
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

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
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

        // Seek to key between existing keys (should find next key)
        iter.seek(b"key_015");
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_020"); // Next key after key_015
    }

    #[test]
    fn test_iterator_seek_before_first() {
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

        // Seek before first key
        iter.seek(b"key_000");
        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"key_001"); // First key
    }

    #[test]
    fn test_iterator_seek_after_last() {
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

        // Seek after last key
        iter.seek(b"key_999");
        assert!(!iter.is_valid()); // No keys >= key_999
    }

    #[test]
    fn test_iterator_seek_last_key() {
        let entries = create_test_entries();
        let mut iter = MemTableIterator::new(entries);

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
        let entries = Vec::new();
        let iter = MemTableIterator::new(entries);

        // Should be invalid from the start
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_single_entry() {
        let entries = vec![(
            b"only_key".to_vec(),
            create_test_record(b"only_key", b"only_value"),
        )];

        let mut iter = MemTableIterator::new(entries);

        assert!(iter.is_valid());
        assert_eq!(iter.key().as_slice(), b"only_key");

        iter.next();
        assert!(!iter.is_valid());
    }

    #[test]
    fn test_iterator_deleted_records() {
        let entries = vec![
            (
                b"key_001".to_vec(),
                create_test_record(b"key_001", b"value_001"),
            ),
            (
                b"key_002".to_vec(),
                LogRecord::tombstone(b"key_002".to_vec()),
            ),
            (
                b"key_003".to_vec(),
                create_test_record(b"key_003", b"value_003"),
            ),
        ];

        let mut iter = MemTableIterator::new(entries);

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
