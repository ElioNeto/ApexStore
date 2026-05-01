// iterator.rs
use crate::memtable::MemTable;
use crate::table::SSTable;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub struct ScanIterator {
    heap: BinaryHeap<ScanItem>,
}

struct ScanItem {
    key: String,
    value: (Vec<u8>, u128, bool),
    source: Source,
}

enum Source {
    MemTable,
    SSTable(usize), // index of SSTable
}

impl PartialEq for ScanItem {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for ScanItem {}

impl PartialOrd for ScanItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Reverse for min-heap (BinaryHeap is max-heap by default)
        other.key.partial_cmp(&self.key)
    }
}

impl Ord for ScanItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key)
    }
}

impl ScanIterator {
    pub fn new(memtable: &MemTable, sstables: &[SSTable]) -> Self {
        let mut heap = BinaryHeap::new();

        // Add all memtable entries
        for (key, value) in memtable.iter() {
            heap.push(ScanItem {
                key: key.clone(),
                value: value.clone(),
                source: Source::MemTable,
            });
        }

        // Add first entry from each SSTable
        for (idx, sstable) in sstables.iter().enumerate() {
            if let Some((key, value)) = sstable.first_key_value() {
                heap.push(ScanItem {
                    key: key.clone(),
                    value: value.clone(),
                    source: Source::SSTable(idx),
                });
            }
        }

        ScanIterator { heap }
    }
}

impl Iterator for ScanIterator {
    type Item = (String, (Vec<u8>, u128, bool));

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.heap.pop()?;
        let key = item.key;
        let value = item.value;

        // Advance the iterator from which this item came
        match item.source {
            Source::MemTable => {
                // MemTable is already fully in heap, nothing more to add
            }
            Source::SSTable(idx) => {
                // Get next entry from this SSTable
                if let Some((next_key, next_value)) = SSTable::next_at_index(idx) {
                    self.heap.push(ScanItem {
                        key: next_key,
                        value: next_value,
                        source: Source::SSTable(idx),
                    });
                }
            }
        }

        Some((key, value))
    }
}
