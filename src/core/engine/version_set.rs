use crate::storage::cache::Cache;

pub struct VersionSet<C: Cache> {
    _cache: std::marker::PhantomData<C>,
}

impl<C: Cache> VersionSet<C> {
    pub fn new(_options: crate::core::engine::EngineOptions, _cache: C) -> Self {
        todo!()
    }

    pub fn get(&self, _cf: &str, _key: &[u8]) -> Option<Vec<u8>> {
        todo!()
    }

    pub fn scan(
        &self,
        _cf: &str,
        _lower: Option<&[u8]>,
        _upper: Option<&[u8]>,
        _limit: Option<usize>,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        todo!()
    }

    pub fn add_table(&mut self, _cf: &str, _table: crate::core::table::Table) {
        todo!()
    }

    pub fn current_version(&self) -> crate::core::version::Version<C> {
        todo!()
    }

    pub fn remove_and_add_table(&mut self, _level: usize, _table: crate::core::table::Table) {
        todo!()
    }
}
