// memtable.rs
impl MemTable {
    pub fn iter(&self) -> impl Iterator<Item = (&String, &(Vec<u8>, u128, bool))> {
        self.map.iter()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}
