use crate::core::engine::{LsmEngine, LsmEngineGeneric};
use crate::storage::cache::Cache;

pub struct SstIterator<C: Cache> {
    engine: LsmEngineGeneric<C>,
}
