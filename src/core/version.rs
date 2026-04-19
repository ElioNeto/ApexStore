use crate::core::engine::LsmEngineGeneric;
use crate::storage::cache::Cache;

pub struct Version<C: Cache> {
    _engine: std::marker::PhantomData<LsmEngineGeneric<C>>,
}

impl<C: Cache> Default for Version<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Cache> Version<C> {
    pub fn new() -> Self {
        Self {
            _engine: std::marker::PhantomData,
        }
    }

    pub fn get_level_tables(&self, _level: usize) -> Vec<crate::core::table::Table> {
        Vec::new()
    }
}
