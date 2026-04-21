use crate::error::Result;
use crate::merge_iterator::MergeIterator;
use crate::record::Record;

const MAX_SCAN_LIMIT: usize = 10_000; // safety bound used by `keys()`

pub struct Engine {
    // fields omitted for brevity
    // e.g., db: Arc<rocksdb::DB>,
}

impl Engine {
    /// Scan all column families (or a single one) with an optional limit.
    pub async fn scan(&self, cf: Option<&str>, limit: Option<usize>) -> Result<Vec<Record>> {
        // If the caller does not provide a limit we must protect ourselves from an
        // unbounded scan.  Use the same hard‑coded safety bound that `keys()` uses.
        let bounded_limit = match limit {
            Some(l) => Some(l),
            None => Some(MAX_SCAN_LIMIT),
        };
        self.scan_cf(cf, None, None, bounded_limit).await
    }

    /// Scan a specific column family with optional start/end bounds and limit.
    /// This method is used internally by `scan` and can also be called directly.
    pub async fn scan_cf(
        &self,
        cf: Option<&str>,
        start: Option<Vec<u8>>,
        end: Option<Vec<u8>>,
        limit: Option<usize>,
    ) -> Result<Vec<Record>> {
        // Build a MergeIterator over the relevant column families.
        let mut iter: MergeIterator = MergeIterator::new(self, cf, start, end, limit).await?;

        let mut records = Vec::new();

        while let Some((key, value)) = iter.next() {
            // Skip tombstone entries.
            if value.is_tombstone() {
                continue;
            }
            records.push(Record::new(key, value));
        }

        Ok(records)
    }

    /// Return all keys (bounded by `MAX_SCAN_LIMIT`).
    pub async fn keys(&self) -> Result<Vec<Vec<u8>>> {
        // Implementation that respects MAX_SCAN_LIMIT.
        // Details omitted for brevity.
        unimplemented!()
    }

    // other methods …
}
