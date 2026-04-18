#[derive(Clone)]
pub struct Table;

impl Table {
    pub fn build(
        _data: std::collections::HashMap<Vec<u8>, Vec<u8>>,
        _options: &crate::core::engine::EngineOptions,
    ) -> Self {
        Self
    }
    pub fn size(&self) -> usize {
        0
    }
    pub fn iter(&self) -> TableIterator {
        TableIterator
    }
}

pub struct TableIterator;
impl crate::core::iterators::StorageIterator for TableIterator {
    type KeyType = crate::core::key::KeySlice<'static>;

    fn next(&mut self) {}
    fn key(&self) -> Self::KeyType {
        crate::core::key::KeySlice::new(&[])
    }
    fn value(&self) -> &[u8] {
        &[]
    }
    fn is_valid(&self) -> bool {
        false
    }
    fn seek(&mut self, _key: &[u8]) {}
}
