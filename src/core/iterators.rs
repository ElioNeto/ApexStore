pub trait StorageIterator {
    type KeyType;

    fn next(&mut self);
    fn key(&self) -> Self::KeyType;
    fn value(&self) -> &[u8];
    fn is_valid(&self) -> bool;
    fn seek(&mut self, key: &[u8]);
}
