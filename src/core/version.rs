use crate::core::engine::LsmEngine;
use crate::storage::cache::Cache;

pub struct Version<C: Cache> {
    _engine: std::marker::PhantomData<LsmEngine<C>>,
}

impl<C: Cache> Version<C> {
    pub fn get_level_tables(&self, _level: usize) -> Vec<crate::core::table::Table> {
        todo!()
    }
}
