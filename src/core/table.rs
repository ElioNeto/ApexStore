#[derive(Clone)]
pub struct Table;

impl Table {
    pub fn build(_data: Vec<(Vec<u8>, Vec<u8>)>, _options: &crate::core::engine::EngineOptions) -> Self {
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
    fn next(&mut self) -> bool { false }
    fn key(&self) -> &[u8] { &[] }
    fn value(&self) -> &[u8] { &[] }
}
