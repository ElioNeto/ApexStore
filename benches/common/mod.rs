use apexstore::infra::error::Result;
use apexstore::LsmEngine;

/// Populate untimed fixtures through bounded batches. Criterion calls its
/// callback repeatedly, so individual WAL syncs during setup can dominate the
/// job duration even when the measured operation takes only milliseconds.
/// Benchmark bodies still exercise their original read/write operations.
pub fn populate<K: AsRef<[u8]>, V: AsRef<[u8]>>(
    engine: &LsmEngine,
    records: impl IntoIterator<Item = (K, V)>,
) -> Result<()> {
    let mut batch = Vec::with_capacity(256);
    for record in records {
        batch.push(record);
        if batch.len() == 256 {
            engine.set_batch(&batch)?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        engine.set_batch(&batch)?;
    }
    Ok(())
}
