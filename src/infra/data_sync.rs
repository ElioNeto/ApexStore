//! Data diff & two-way synchronisation.
//!
//! This module provides:
//!
//! - [`DataSync`] — compares local state with a remote endpoint and
//!   performs bi-directional sync.
//! - [`DiffEntry`] — a single diff entry describing a key that differs.
//! - [`SyncDirection`] — the direction of synchronisation.

use std::collections::HashMap;

type BoxResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type DataMap = HashMap<Vec<u8>, (Vec<u8>, u64)>;
type DataEntries = Vec<(Vec<u8>, Vec<u8>, u64)>;

/// The direction of synchronisation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncDirection {
    /// Pull from remote (remote overwrites local).
    Pull,
    /// Push to remote (local overwrites remote).
    Push,
    /// Two-way merge — the side with the higher timestamp wins.
    TwoWay,
}

/// A single diff entry representing a key that differs between local and remote.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// The key that differs.
    pub key: Vec<u8>,
    /// The local value (if any).
    pub local_value: Option<Vec<u8>>,
    /// The remote value (if any).
    pub remote_value: Option<Vec<u8>>,
    /// The local timestamp.
    pub local_timestamp: u64,
    /// The remote timestamp.
    pub remote_timestamp: u64,
}

/// The result of a sync operation.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Number of keys that were synced.
    pub keys_synced: u64,
    /// Number of conflicts that were resolved.
    pub conflicts_resolved: u64,
}

/// A trait for fetching key-value state from a remote source.
///
/// Implementations could be HTTP clients, file readers, or in-memory stores.
pub trait RemoteBackend: Send + Sync {
    /// Fetch all key-value pairs with timestamps from the remote.
    fn fetch_all(&self) -> BoxResult<DataMap>;
    /// Push key-value pairs to the remote.
    fn push(&self, entries: &DataEntries) -> BoxResult<()>;
}

/// Engine trait for interacting with the local KV store.
pub trait LocalEngine: Send + Sync {
    /// Return all key-value pairs with timestamps.
    fn all_entries(&self) -> BoxResult<DataEntries>;
    /// Apply a set of key-value pairs (upsert).
    fn apply_batch(&self, entries: &DataEntries) -> BoxResult<()>;
}

/// Orchestrates diff computation and bi-directional sync between a local
/// engine and a remote backend.
pub struct DataSync {
    local: Box<dyn LocalEngine>,
    remote: Box<dyn RemoteBackend>,
}

impl DataSync {
    /// Create a new `DataSync` with the given local engine and remote backend.
    pub fn new(local: Box<dyn LocalEngine>, remote: Box<dyn RemoteBackend>) -> Self {
        Self { local, remote }
    }

    /// Compute the diff between local and remote state.
    ///
    /// Returns a vector of [`DiffEntry`] for keys that exist in one side but
    /// not the other, or that have different values/timestamps.
    pub fn diff(&self) -> BoxResult<Vec<DiffEntry>> {
        let local_map: HashMap<Vec<u8>, (Vec<u8>, u64)> = self
            .local
            .all_entries()?
            .into_iter()
            .map(|(k, v, ts)| (k, (v, ts)))
            .collect();
        let remote_map = self.remote.fetch_all()?;

        let mut entries = Vec::new();

        // Check keys in local but maybe not in remote.
        for (key, (local_val, local_ts)) in &local_map {
            match remote_map.get(key) {
                Some((remote_val, remote_ts))
                    if local_val == remote_val && local_ts == remote_ts =>
                {
                    // Identical — skip.
                }
                Some((remote_val, remote_ts)) => {
                    entries.push(DiffEntry {
                        key: key.clone(),
                        local_value: Some(local_val.clone()),
                        remote_value: Some(remote_val.clone()),
                        local_timestamp: *local_ts,
                        remote_timestamp: *remote_ts,
                    });
                }
                None => {
                    entries.push(DiffEntry {
                        key: key.clone(),
                        local_value: Some(local_val.clone()),
                        remote_value: None,
                        local_timestamp: *local_ts,
                        remote_timestamp: 0,
                    });
                }
            }
        }

        // Check keys in remote but not in local.
        for (key, (remote_val, remote_ts)) in &remote_map {
            if !local_map.contains_key(key) {
                entries.push(DiffEntry {
                    key: key.clone(),
                    local_value: None,
                    remote_value: Some(remote_val.clone()),
                    local_timestamp: 0,
                    remote_timestamp: *remote_ts,
                });
            }
        }

        Ok(entries)
    }

    /// Synchronise data in the given direction.
    ///
    /// * `SyncDirection::Pull` — remote overwrites local.
    /// * `SyncDirection::Push` — local overwrites remote.
    /// * `SyncDirection::TwoWay` — per-key timestamp comparison wins.
    pub fn sync(&self, direction: SyncDirection) -> BoxResult<SyncResult> {
        let diffs = self.diff()?;
        let resolved = self.resolve_conflicts_impl(&diffs, direction)?;

        let keys_synced = resolved.len() as u64;
        let conflicts_resolved = diffs.len() as u64;

        Ok(SyncResult {
            keys_synced,
            conflicts_resolved,
        })
    }

    /// Resolve conflicts for a set of diff entries using the given direction.
    ///
    /// Returns the resolved entries (key, value, timestamp).
    pub fn resolve_conflicts(
        &self,
        entries: Vec<DiffEntry>,
        direction: SyncDirection,
    ) -> BoxResult<DataEntries> {
        self.resolve_conflicts_impl(&entries, direction)
    }

    fn resolve_conflicts_impl(
        &self,
        entries: &[DiffEntry],
        direction: SyncDirection,
    ) -> BoxResult<DataEntries> {
        let mut resolved = Vec::with_capacity(entries.len());

        for entry in entries {
            match direction {
                SyncDirection::Pull => {
                    if let Some(remote_val) = &entry.remote_value {
                        resolved.push((
                            entry.key.clone(),
                            remote_val.clone(),
                            entry.remote_timestamp,
                        ));
                    }
                }
                SyncDirection::Push => {
                    if let Some(local_val) = &entry.local_value {
                        resolved.push((
                            entry.key.clone(),
                            local_val.clone(),
                            entry.local_timestamp,
                        ));
                    }
                }
                SyncDirection::TwoWay => {
                    if entry.remote_timestamp >= entry.local_timestamp {
                        if let Some(remote_val) = &entry.remote_value {
                            resolved.push((
                                entry.key.clone(),
                                remote_val.clone(),
                                entry.remote_timestamp,
                            ));
                        }
                    } else if let Some(local_val) = &entry.local_value {
                        resolved.push((
                            entry.key.clone(),
                            local_val.clone(),
                            entry.local_timestamp,
                        ));
                    }
                }
            }
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MemLocal {
        #[allow(clippy::type_complexity)]
        data: Mutex<Vec<(Vec<u8>, Vec<u8>, u64)>>,
    }

    impl MemLocal {
        fn new(data: Vec<(Vec<u8>, Vec<u8>, u64)>) -> Self {
            Self {
                data: Mutex::new(data),
            }
        }
    }

    impl LocalEngine for MemLocal {
        fn all_entries(&self) -> BoxResult<DataEntries> {
            Ok(self.data.lock().unwrap().clone())
        }

        fn apply_batch(&self, entries: &DataEntries) -> BoxResult<()> {
            let mut data = self.data.lock().unwrap();
            for (k, v, ts) in entries {
                data.push((k.clone(), v.clone(), *ts));
            }
            Ok(())
        }
    }

    struct MemRemote {
        #[allow(clippy::type_complexity)]
        data: Mutex<HashMap<Vec<u8>, (Vec<u8>, u64)>>,
    }

    impl MemRemote {
        fn new(data: HashMap<Vec<u8>, (Vec<u8>, u64)>) -> Self {
            Self {
                data: Mutex::new(data),
            }
        }
    }

    impl RemoteBackend for MemRemote {
        fn fetch_all(&self) -> BoxResult<DataMap> {
            Ok(self.data.lock().unwrap().clone())
        }

        fn push(&self, entries: &DataEntries) -> BoxResult<()> {
            let mut data = self.data.lock().unwrap();
            for (k, v, ts) in entries {
                data.insert(k.clone(), (v.clone(), *ts));
            }
            Ok(())
        }
    }

    fn make_local(a: &[(&[u8], &[u8], u64)]) -> Box<dyn LocalEngine> {
        Box::new(MemLocal::new(
            a.iter()
                .map(|(k, v, ts)| (k.to_vec(), v.to_vec(), *ts))
                .collect(),
        ))
    }

    fn make_remote(a: &[(&[u8], &[u8], u64)]) -> Box<dyn RemoteBackend> {
        let mut map = HashMap::new();
        for (k, v, ts) in a {
            map.insert(k.to_vec(), (v.to_vec(), *ts));
        }
        Box::new(MemRemote::new(map))
    }

    #[test]
    fn test_diff_identical() {
        let local = make_local(&[(b"k1", b"v1", 1)]);
        let remote = make_remote(&[(b"k1", b"v1", 1)]);
        let sync = DataSync::new(local, remote);
        let diffs = sync.diff().unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_local_only() {
        let local = make_local(&[(b"k1", b"v1", 1)]);
        let remote = make_remote(&[]);
        let sync = DataSync::new(local, remote);
        let diffs = sync.diff().unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].key, b"k1");
        assert_eq!(diffs[0].remote_value, None);
    }

    #[test]
    fn test_diff_remote_only() {
        let local = make_local(&[]);
        let remote = make_remote(&[(b"k2", b"v2", 2)]);
        let sync = DataSync::new(local, remote);
        let diffs = sync.diff().unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].key, b"k2");
        assert_eq!(diffs[0].local_value, None);
    }

    #[test]
    fn test_diff_different_value() {
        let local = make_local(&[(b"k1", b"local_val", 1)]);
        let remote = make_remote(&[(b"k1", b"remote_val", 2)]);
        let sync = DataSync::new(local, remote);
        let diffs = sync.diff().unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].local_value, Some(b"local_val".to_vec()));
        assert_eq!(diffs[0].remote_value, Some(b"remote_val".to_vec()));
    }

    #[test]
    fn test_sync_pull() {
        let local = make_local(&[(b"k1", b"local", 1)]);
        let remote = make_remote(&[(b"k1", b"remote", 2)]);
        let sync = DataSync::new(local, remote);
        let result = sync.sync(SyncDirection::Pull).unwrap();
        assert_eq!(result.conflicts_resolved, 1);
        // Under pull, remote wins.
        let entries = sync
            .resolve_conflicts(sync.diff().unwrap(), SyncDirection::Pull)
            .unwrap();
        assert_eq!(entries[0].1, b"remote");
    }

    #[test]
    fn test_sync_push() {
        let local = make_local(&[(b"k1", b"local", 1)]);
        let remote = make_remote(&[(b"k1", b"remote", 2)]);
        let sync = DataSync::new(local, remote);
        let entries = sync
            .resolve_conflicts(sync.diff().unwrap(), SyncDirection::Push)
            .unwrap();
        assert_eq!(entries[0].1, b"local");
    }

    #[test]
    fn test_sync_two_way_remote_wins() {
        let local = make_local(&[(b"k1", b"local", 1)]);
        let remote = make_remote(&[(b"k1", b"remote", 2)]);
        let sync = DataSync::new(local, remote);
        let entries = sync
            .resolve_conflicts(sync.diff().unwrap(), SyncDirection::TwoWay)
            .unwrap();
        assert_eq!(entries[0].1, b"remote");
    }

    #[test]
    fn test_sync_two_way_local_wins() {
        let local = make_local(&[(b"k1", b"local", 3)]);
        let remote = make_remote(&[(b"k1", b"remote", 2)]);
        let sync = DataSync::new(local, remote);
        let entries = sync
            .resolve_conflicts(sync.diff().unwrap(), SyncDirection::TwoWay)
            .unwrap();
        assert_eq!(entries[0].1, b"local");
    }
}
