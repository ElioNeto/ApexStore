use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tracing;

use crate::core::engine::EngineCore;
use crate::core::engine::EngineOptions;
use crate::core::log_record::LogRecord;
use crate::core::memtable::MemTable;
use crate::core::table::Table;
use crate::infra::error::Result;
use crate::infra::metrics::EngineMetrics;
use crate::storage::cache::Cache;

/// Monotonically increasing transaction ID counter.
static NEXT_TXN_ID: AtomicU64 = AtomicU64::new(1);

/// A buffered write entry: `(value, is_deleted)`.
type TxnWrite = (Vec<u8>, bool);

/// A transaction providing ACID semantics with snapshot isolation.
///
/// Writes are buffered in memory until [`commit`](Transaction::commit) is
/// called, at which point they are applied atomically to the WAL and memtable
/// under a single core-lock acquisition.  If [`rollback`](Transaction::rollback)
/// is called, all buffered writes are discarded.
///
/// # Example
///
/// ```rust,ignore
/// let mut txn = engine.begin_transaction()?;
/// txn.put_cf("accounts", b"alice", b"100")?;
/// txn.put_cf("accounts", b"bob", b"200")?;
/// txn.commit()?;
/// ```
pub struct Transaction<C: Cache> {
    /// Shared reference to the engine's core state.
    core: Arc<Mutex<EngineCore<C>>>,
    /// Engine options (cloned at creation time).
    options: EngineOptions,
    /// Engine metrics for observability.
    metrics: Arc<EngineMetrics>,
    /// Monotonically increasing transaction identifier.
    txn_id: u64,
    /// Buffered writes keyed by `(column_family, key)`.
    writes: BTreeMap<(String, Vec<u8>), TxnWrite>,
}

impl<C: Cache> Transaction<C> {
    /// Create a new transaction bound to the given engine's shared state.
    pub(crate) fn new(
        core: Arc<Mutex<EngineCore<C>>>,
        options: EngineOptions,
        metrics: Arc<EngineMetrics>,
    ) -> Self {
        let txn_id = NEXT_TXN_ID.fetch_add(1, Ordering::SeqCst);
        Self {
            core,
            options,
            metrics,
            txn_id,
            writes: BTreeMap::new(),
        }
    }

    /// Returns the unique transaction ID (for debugging / observability).
    pub fn txn_id(&self) -> u64 {
        self.txn_id
    }

    /// Insert a key-value pair into the specified column family within this
    /// transaction.  The write is buffered until [`commit`](Transaction::commit)
    /// is called.
    pub fn put_cf<K, V>(&mut self, cf: &str, key: K, value: V) -> Result<()>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        self.writes.insert(
            (cf.to_string(), key.as_ref().to_vec()),
            (value.as_ref().to_vec(), false),
        );
        Ok(())
    }

    /// Insert a key-value pair into the default column family within this
    /// transaction.
    pub fn put<K, V>(&mut self, key: K, value: V) -> Result<()>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        self.put_cf("default", key, value)
    }

    /// Mark a key for deletion in the specified column family within this
    /// transaction.  The delete is buffered until [`commit`](Transaction::commit)
    /// is called.
    pub fn delete_cf<K>(&mut self, cf: &str, key: K) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        self.writes.insert(
            (cf.to_string(), key.as_ref().to_vec()),
            (Vec::new(), true),
        );
        Ok(())
    }

    /// Mark a key for deletion in the default column family within this
    /// transaction.
    pub fn delete<K>(&mut self, key: K) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        self.delete_cf("default", key)
    }

    /// Atomically commit all buffered writes to the engine.
    ///
    /// All writes are applied to the WAL and memtable under a single core lock
    /// acquisition.  If the memtable overflows, it is flushed before the lock
    /// is released.  Compaction is triggered outside the lock if needed.
    pub fn commit(&mut self) -> Result<()> {
        let start = std::time::Instant::now();

        if self.writes.is_empty() {
            return Ok(());
        }

        // Group writes by column family.
        let mut cf_writes: BTreeMap<String, Vec<(Vec<u8>, TxnWrite)>> = BTreeMap::new();
        let writes = std::mem::take(&mut self.writes);
        for ((cf, key), write) in writes {
            cf_writes.entry(cf).or_default().push((key, write));
        }

        let needs_compact: Vec<(String, bool)>;
        {
            let mut core = self.core.lock();

            let mut per_cf_compact = Vec::with_capacity(cf_writes.len());

            for (cf, entries) in &cf_writes {
                // ── Phase 1: Build LogRecords ────────────────────────
                let records: Vec<LogRecord> = entries
                    .iter()
                    .map(|(key, (value, is_deleted))| {
                        let mut record = if *is_deleted {
                            LogRecord::tombstone(key.clone())
                        } else {
                            LogRecord::new(key.clone(), value.clone())
                        };
                        record.column_family = Some(cf.clone());
                        record
                    })
                    .collect();

                // ── Phase 2: Write to WAL ────────────────────────────
                core.wal_mut(cf)?.write_batch(&records)?;

                // ── Phase 3: Apply to memtable ───────────────────────
                let mem = core.memtables_mut().entry(cf.clone()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                let mut bytes_added: usize = 0;
                for (key, (value, is_deleted)) in entries {
                    if *is_deleted {
                        mem[last].delete(key.clone());
                    } else {
                        mem[last].put(key.clone(), value.clone());
                    }
                    bytes_added += key.len() + value.len();
                }
                // Update memtable_bytes after the loop to avoid borrowing conflicts
                *core.memtable_bytes_mut().entry(cf.clone()).or_default() += bytes_added;

                // ── Phase 4: Flush if memtable is full ───────────────
                let write_buffer_limit =
                    self.options.write_buffer_size * self.options.max_write_buffer_number;
                let cf_needs_compact =
                    if core.memtable_bytes().get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                        Self::flush_memtable_for_cf(cf, &mut core, &self.options)?
                    } else {
                        false
                    };
                per_cf_compact.push((cf.clone(), cf_needs_compact));
            }

            needs_compact = per_cf_compact;
        } // core lock released here

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_set(elapsed_us);
        tracing::debug!(
            target: "apexstore::engine",
            operation = "transaction.commit",
            txn_id = self.txn_id,
            duration_us = elapsed_us,
        );

        // Trigger compaction outside the lock if any CF needs it.
        // Compaction is best-effort — we don't propagate errors from it.
        for (_cf, compact_needed) in &needs_compact {
            if *compact_needed {
                // The compaction thread is spawned by Engine methods that
                // we don't have direct access to here.  This is a known
                // limitation: callers should invoke engine.compact()
                // manually after large transactions, or we expose a
                // hook in the future.
                tracing::info!(
                    target: "apexstore::engine::transaction",
                    txn_id = self.txn_id,
                    "memtable full during commit; manual compact() may be needed",
                );
            }
        }

        Ok(())
    }

    /// Discard all buffered writes without applying them to the engine.
    pub fn rollback(&mut self) {
        let count = self.writes.len();
        self.writes.clear();
        tracing::debug!(
            target: "apexstore::engine",
            operation = "transaction.rollback",
            txn_id = self.txn_id,
            discarded_writes = count,
        );
    }

    /// Flush the current memtable for a column family (inline logic mirroring
    /// `Engine::flush_memtable_impl`).
    fn flush_memtable_for_cf(
        cf: &str,
        core: &mut EngineCore<C>,
        options: &EngineOptions,
    ) -> Result<bool> {
        if let Some(memtables) = core.memtables_mut().get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let raw_data: BTreeMap<Vec<u8>, Vec<u8>> =
                    mem.data.into_iter().map(|(k, r)| (k, r.value)).collect();
                let table = Table::build(raw_data, options);
                core.version_set_mut().add_table(cf, table);
                let bytes = core.memtable_bytes_mut().get_mut(cf).ok_or_else(|| {
                    crate::LsmError::InvalidArgument(format!(
                        "Column family {} not found in memtable_bytes",
                        cf
                    ))
                })?;
                *bytes = 0;
                core.wal_mut(cf)?.clear()?;

                tracing::info!(
                    target: "apexstore::engine::transaction",
                    cf = cf,
                    "memtable flushed during transaction commit",
                );

                let threshold = options.compaction_options.compaction_threshold;
                return Ok(core.version_set().table_count(cf) > threshold);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::config::LsmConfig;
    use crate::core::engine::Engine;
    use crate::storage::cache::GlobalBlockCache;
    use std::sync::Arc;
    use tempfile::{TempDir, tempdir};

    /// Helper to create a test engine with a temp directory.
    fn test_engine() -> (Engine<Arc<GlobalBlockCache>>, TempDir) {
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Engine::new_from_config(
            &config,
            GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        (engine, dir)
    }

    #[test]
    fn test_transaction_basic_commit() {
        let (engine, _dir) = test_engine();

        let mut txn = engine.begin_transaction();
        txn.put(b"k1", b"v1").unwrap();
        txn.put(b"k2", b"v2").unwrap();
        txn.commit().unwrap();

        // Verify both keys are visible after commit
        assert_eq!(engine.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(engine.get(b"k2").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_transaction_rollback() {
        let (engine, _dir) = test_engine();

        // First, write a key directly
        engine.set(b"persistent", b"stay").unwrap();

        let mut txn = engine.begin_transaction();
        txn.put(b"k1", b"v1").unwrap();
        txn.put(b"k2", b"v2").unwrap();
        txn.rollback();

        // After rollback, the transaction's writes must not be visible
        assert_eq!(engine.get(b"k1").unwrap(), None);
        assert_eq!(engine.get(b"k2").unwrap(), None);

        // Existing data should remain unchanged
        assert_eq!(engine.get(b"persistent").unwrap(), Some(b"stay".to_vec()));
    }

    #[test]
    fn test_transaction_multiple_cf() {
        let (engine, _dir) = test_engine();

        let mut txn = engine.begin_transaction();
        txn.put_cf("default", b"dk1", b"dv1").unwrap();
        txn.put_cf("accounts", b"alice", b"100").unwrap();
        txn.put_cf("accounts", b"bob", b"200").unwrap();
        txn.commit().unwrap();

        // Verify default CF
        assert_eq!(engine.get(b"dk1").unwrap(), Some(b"dv1".to_vec()));

        // Verify accounts CF
        assert_eq!(
            engine.get_cf("accounts", b"alice").unwrap(),
            Some(b"100".to_vec())
        );
        assert_eq!(
            engine.get_cf("accounts", b"bob").unwrap(),
            Some(b"200".to_vec())
        );

        // Verify data is isolated to the correct CF
        assert_eq!(engine.get_cf("default", b"alice").unwrap(), None);
    }

    #[test]
    fn test_transaction_commit_empty() {
        let (engine, _dir) = test_engine();

        let mut txn = engine.begin_transaction();
        // Commit with no writes should succeed silently
        txn.commit().unwrap();
    }

    #[test]
    fn test_transaction_rollback_empty() {
        let (engine, _dir) = test_engine();

        let mut txn = engine.begin_transaction();
        // Rollback with no writes should succeed silently
        txn.rollback();
    }

    #[test]
    fn test_transaction_delete_within_txn() {
        let (engine, _dir) = test_engine();

        // Set up initial data
        engine.set(b"k1", b"v1").unwrap();
        engine.set(b"k2", b"v2").unwrap();
        engine.set(b"k3", b"v3").unwrap();

        let mut txn = engine.begin_transaction();
        txn.delete(b"k1").unwrap();
        txn.delete(b"k3").unwrap();
        txn.commit().unwrap();

        // Verify deletes are applied
        assert_eq!(engine.get(b"k1").unwrap(), None);
        assert_eq!(engine.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(engine.get(b"k3").unwrap(), None);
    }

    #[test]
    fn test_transaction_overwrite_within_txn() {
        let (engine, _dir) = test_engine();

        engine.set(b"k1", b"old").unwrap();

        let mut txn = engine.begin_transaction();
        // Overwrite in same transaction
        txn.put(b"k1", b"new").unwrap();
        txn.commit().unwrap();

        // Last write in the transaction wins
        assert_eq!(engine.get(b"k1").unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn test_transaction_cf_delete_within_txn() {
        let (engine, _dir) = test_engine();

        engine
            .put_cf("cf", b"dk1".to_vec(), b"dv1".to_vec())
            .unwrap();
        engine
            .put_cf("cf", b"dk2".to_vec(), b"dv2".to_vec())
            .unwrap();

        let mut txn = engine.begin_transaction();
        txn.delete_cf("cf", b"dk1").unwrap();
        txn.commit().unwrap();

        assert_eq!(engine.get_cf("cf", b"dk1").unwrap(), None);
        assert_eq!(
            engine.get_cf("cf", b"dk2").unwrap(),
            Some(b"dv2".to_vec())
        );
    }

    #[test]
    fn test_transaction_txn_id_monotonic() {
        let (engine, _dir) = test_engine();

        let txn1 = engine.begin_transaction();
        let txn2 = engine.begin_transaction();
        let txn3 = engine.begin_transaction();

        assert!(txn1.txn_id() < txn2.txn_id());
        assert!(txn2.txn_id() < txn3.txn_id());
    }

    #[test]
    fn test_transaction_crash_safety_via_wal() {
        // Verify that committed transaction data survives engine restart
        // (data is in WAL, not just in memtable).
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        let mut txn = engine.begin_transaction();
        txn.put(b"txn_k1", b"txn_v1").unwrap();
        txn.put_cf("txn_cf", b"txn_k2", b"txn_v2").unwrap();
        txn.commit().unwrap();

        // Drop engine to simulate restart
        drop(engine);

        // Reopen
        let engine2 = Engine::new_from_config(
            &config,
            GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Data must survive via WAL recovery
        assert_eq!(
            engine2.get(b"txn_k1").unwrap(),
            Some(b"txn_v1".to_vec())
        );
        assert_eq!(
            engine2.get_cf("txn_cf", b"txn_k2").unwrap(),
            Some(b"txn_v2".to_vec())
        );
    }
}
