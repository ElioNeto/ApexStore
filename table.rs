// table.rs
impl SSTable {
    pub fn record_count(&self) -> usize {
        self.metadata.record_count as usize
    }

    pub fn first_key_value(&self) -> Option<(String, (Vec<u8>, u128, bool))> {
        self.iter().next()
    }

    // Static method to get next item from specific SSTable by index
    // This would need a way to track position per SSTable iterator
    // Alternative: store iterators in ScanIterator instead of indices
}
