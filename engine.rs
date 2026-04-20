// engine.rs
use crate::iterator::ScanIterator;

// ... existing code ...

pub fn scan(&self) -> ScanIterator {
    ScanIterator::new(&self.memtable, &self.sstables)
}

pub fn keys(&self) -> Vec<String> {
    const MAX_SCAN_LIMIT: usize = 1000; // or configurable
    self.scan()
        .take(MAX_SCAN_LIMIT)
        .map(|(key, _)| key)
        .collect()
}

pub fn count(&self) -> usize {
    let memtable_count = self.memtable.len();
    let sstable_count: usize = self.sstables.iter().map(|s| s.record_count()).sum();
    memtable_count + sstable_count
}

// search() also needs updating to use iterator pattern
pub fn search(&self, query: &str) -> Vec<(String, (Vec<u8>, u128, bool))> {
    self.scan()
        .take(1000) // also cap search results
        .filter(|(_, (_, _, deleted))| !*deleted)
        .filter(|(_, (value, _, _))| String::from_utf8_lossy(value).contains(query))
        .collect()
}
