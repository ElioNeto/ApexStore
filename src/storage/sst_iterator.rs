use crate::core::engine::LsmEngineGeneric;
use crate::storage::cache::Cache;

pub struct SstIterator<C: Cache> {
    _engine: LsmEngineGeneric<C>,
}
