use crate::core::engine::LsmEngine;
use crate::storage::cache::Cache;

pub struct SstIterator<C: Cache> {
    engine: LsmEngine<C>,
}
