//! Time-travel queries — query the store as it appeared at a past point in time.
//!
//! [`TimeTravelEngine`] keeps historical snapshots (key-value pairs annotated
//! with timestamps) and allows querying the data as it existed at a given
//! moment or within a time window.
//!
//! Snapshots can be persisted to disk as JSON files and restored on startup,
//! enabling time-travel queries across process restarts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── Snapshot ────────────────────────────────────────────────────────────────

/// A snapshot of engine state captured at a specific instant, persistable to
/// disk via JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Monotonic timestamp (nanoseconds since Unix epoch).
    timestamp: u128,
    /// All key-value pairs at that moment (stored as hex-encoded entries for
    /// JSON compatibility).
    #[serde(with = "hex_map_serde")]
    data: HashMap<Vec<u8>, Vec<u8>>,
    /// Human-readable label for the snapshot.
    label: String,
}

// ── Custom serde for HashMap<Vec<u8>, Vec<u8>> ──────────────────────────────

/// Serialises a `HashMap<Vec<u8>, Vec<u8>>` as a JSON array of objects with
/// hex-encoded keys and values.
mod hex_map_serde {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::HashMap;

    #[derive(Serialize, Deserialize)]
    struct HexEntry {
        key: String,
        value: String,
    }

    pub fn serialize<S>(data: &HashMap<Vec<u8>, Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let entries: Vec<HexEntry> = data
            .iter()
            .map(|(k, v)| HexEntry {
                key: hex::encode(k),
                value: hex::encode(v),
            })
            .collect();
        let mut seq = serializer.serialize_seq(Some(entries.len()))?;
        for entry in &entries {
            seq.serialize_element(entry)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<Vec<u8>, Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries: Vec<HexEntry> = Vec::deserialize(deserializer)?;
        let mut map = HashMap::new();
        for entry in entries {
            let k = hex::decode(&entry.key).map_err(D::Error::custom)?;
            let v = hex::decode(&entry.value).map_err(D::Error::custom)?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

// ── TimeTravelEngine ────────────────────────────────────────────────────────

/// Engine for time-travel queries with optional disk persistence.
///
/// Snapshots are stored in memory and, when configured with a `base_path`,
/// are also persisted to disk as JSON files (`{base_path}/snapshot_{timestamp}.json`).
/// On startup, call [`restore_snapshots`](TimeTravelEngine::restore_snapshots) to
/// load previously persisted snapshots back into memory.
pub struct TimeTravelEngine {
    /// All captured snapshots, sorted by timestamp (oldest first).
    snapshots: Vec<Snapshot>,
    /// Maximum number of snapshots to retain.
    max_snapshots: usize,
    /// Optional base path for disk persistence.
    base_path: Option<PathBuf>,
}

impl TimeTravelEngine {
    /// Create a new time-travel engine with the given capacity.
    ///
    /// `max_snapshots` limits how many historical snapshots are kept.
    /// When the limit is exceeded, the oldest snapshots are evicted.
    /// Snapshots are kept in memory only.
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
            base_path: None,
        }
    }

    /// Create a new time-travel engine with disk persistence.
    ///
    /// Snapshots will be written to `{base_path}/snapshot_{timestamp}.json`
    /// and can be restored on startup via [`restore_snapshots`](TimeTravelEngine::restore_snapshots).
    pub fn new_with_persistence(max_snapshots: usize, base_path: PathBuf) -> Self {
        // Ensure the directory exists.
        let _ = std::fs::create_dir_all(&base_path);
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
            base_path: Some(base_path),
        }
    }

    /// Capture the current engine state as a snapshot.
    ///
    /// `data` should be a full dump of the column family at this instant.
    /// `label` is an optional human-readable name for the snapshot.
    ///
    /// If persistence is enabled (via [`new_with_persistence`]), the snapshot
    /// is also written to disk.
    pub fn capture(&mut self, data: HashMap<Vec<u8>, Vec<u8>>, label: &str) -> u128 {
        let timestamp = now_nanos();

        let snapshot = Snapshot {
            timestamp,
            data,
            label: label.to_string(),
        };

        // Persist to disk if configured.
        if self.base_path.is_some() {
            let _ = self.persist_snapshot(&snapshot);
        }

        self.snapshots.push(snapshot);

        // Evict oldest snapshots if over capacity.
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.remove(0);
        }

        timestamp
    }

    /// Query a key's value as of the given timestamp.
    ///
    /// Returns the value from the most recent snapshot at or before
    /// `timestamp`.  Returns `None` if no snapshot exists at or before
    /// that time, or if the key was not present in the snapshot.
    pub fn query_as_of(&self, key: &[u8], timestamp: u128) -> Option<Vec<u8>> {
        self.snapshot_at_or_before(timestamp)
            .and_then(|snap| snap.data.get(key).cloned())
    }

    /// Query all key-value pairs that existed within `(start_ts, end_ts]`.
    ///
    /// Returns data from the snapshot closest to `end_ts` but not after it.
    /// If no snapshot falls within the range, returns `None`.
    pub fn query_range(&self, start_ts: u128, end_ts: u128) -> Option<HashMap<Vec<u8>, Vec<u8>>> {
        let snapshot = self.snapshot_at_or_before(end_ts)?;
        if snapshot.timestamp < start_ts {
            return None;
        }
        Some(snapshot.data.clone())
    }

    /// List all snapshots with their timestamps and labels.
    pub fn list_snapshots(&self) -> Vec<(u128, &str)> {
        self.snapshots
            .iter()
            .map(|s| (s.timestamp, s.label.as_str()))
            .collect()
    }

    /// Return the number of stored snapshots.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Remove a snapshot at the given timestamp (if it exists).
    pub fn remove_snapshot(&mut self, timestamp: u128) -> bool {
        let pos = self.snapshots.iter().position(|s| s.timestamp == timestamp);
        if let Some(idx) = pos {
            self.snapshots.remove(idx);
            true
        } else {
            false
        }
    }

    /// Clear all snapshots from memory.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }

    // ── Persistence methods ───────────────────────────────────────────────

    /// Write a single snapshot to disk as a JSON file.
    ///
    /// The file is placed at `{base_path}/snapshot_{timestamp}.json`.
    /// Returns an error if serialization or I/O fails.
    pub fn persist_snapshot(&self, snapshot: &Snapshot) -> Result<(), String> {
        let base = self.base_path.as_ref().ok_or_else(|| {
            "Persistence not configured: no base_path set".to_string()
        })?;
        let filename = format!("snapshot_{}.json", snapshot.timestamp);
        let path = base.join(&filename);
        let json = serde_json::to_string_pretty(snapshot)
            .map_err(|e| format!("Failed to serialize snapshot: {}", e))?;
        std::fs::write(&path, &json)
            .map_err(|e| format!("Failed to write snapshot file: {}", e))?;
        Ok(())
    }

    /// Load snapshot file paths from the persistence directory.
    ///
    /// Returns a sorted list of `(timestamp, PathBuf)` pairs.
    pub fn load_snapshots(&self) -> Result<Vec<(u128, PathBuf)>, String> {
        let base = self.base_path.as_ref().ok_or_else(|| {
            "Persistence not configured: no base_path set".to_string()
        })?;
        let mut entries = Vec::new();

        let dir = std::fs::read_dir(base)
            .map_err(|e| format!("Failed to read persistence directory: {}", e))?;

        for entry in dir {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(ts_str) = filename
                    .strip_prefix("snapshot_")
                    .and_then(|s| s.strip_suffix(".json"))
                {
                    if let Ok(timestamp) = ts_str.parse::<u128>() {
                        entries.push((timestamp, path));
                    }
                }
            }
        }

        // Sort by timestamp ascending.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    /// Restore snapshots from disk into memory.
    ///
    /// Reads all `snapshot_{timestamp}.json` files from the persistence
    /// directory, deserialises them, and adds them to the in-memory list.
    /// Oldest snapshots are evicted if the total exceeds `max_snapshots`.
    pub fn restore_snapshots(&mut self) -> Result<usize, String> {
        let entries = self.load_snapshots()?;
        let mut restored = 0usize;

        for (_ts, path) in entries {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read snapshot file: {}", e))?;
            let snapshot: Snapshot = serde_json::from_str(&raw)
                .map_err(|e| format!("Failed to deserialize snapshot: {}", e))?;

            // Avoid duplicates (same timestamp already in memory).
            if !self.snapshots.iter().any(|s| s.timestamp == snapshot.timestamp) {
                self.snapshots.push(snapshot);
                restored += 1;
            }
        }

        // Re-sort by timestamp and evict if over capacity.
        self.snapshots.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        while self.snapshots.len() > self.max_snapshots {
            self.snapshots.remove(0);
        }

        Ok(restored)
    }

    /// Return the current persistence base path, if any.
    pub fn base_path(&self) -> Option<&PathBuf> {
        self.base_path.as_ref()
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    /// Find the most recent snapshot at or before `timestamp`.
    fn snapshot_at_or_before(&self, timestamp: u128) -> Option<&Snapshot> {
        self.snapshots
            .iter()
            .filter(|s| s.timestamp <= timestamp)
            .max_by_key(|s| s.timestamp)
    }
}

/// Returns the current time in nanoseconds since the Unix epoch.
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_data(pairs: &[(&[u8], &[u8])]) -> HashMap<Vec<u8>, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect()
    }

    // ── In-memory tests (existing) ────────────────────────────────────────

    #[test]
    fn test_capture_and_query_as_of() {
        let mut engine = TimeTravelEngine::new(10);

        let ts1 = engine.capture(make_data(&[(b"a", b"1"), (b"b", b"2")]), "snap1");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let ts2 = engine.capture(make_data(&[(b"a", b"10"), (b"c", b"3")]), "snap2");

        // Query older snapshot
        assert_eq!(engine.query_as_of(b"a", ts1), Some(b"1".to_vec()));
        assert_eq!(engine.query_as_of(b"b", ts1), Some(b"2".to_vec()));
        assert_eq!(engine.query_as_of(b"c", ts1), None);

        // Query newer snapshot
        assert_eq!(engine.query_as_of(b"a", ts2), Some(b"10".to_vec()));
        assert_eq!(engine.query_as_of(b"c", ts2), Some(b"3".to_vec()));
        assert_eq!(engine.query_as_of(b"b", ts2), None); // removed in snap2
    }

    #[test]
    fn test_query_as_of_no_snapshot() {
        let engine = TimeTravelEngine::new(5);
        assert_eq!(engine.query_as_of(b"x", 0), None);
    }

    #[test]
    fn test_query_range() {
        let mut engine = TimeTravelEngine::new(10);

        let ts1 = engine.capture(make_data(&[(b"a", b"1")]), "snap1");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let ts2 = engine.capture(make_data(&[(b"a", b"2")]), "snap2");

        // Range that covers both snapshots should return snap2 (closest to end)
        let result = engine.query_range(ts1, ts2 + 1).unwrap();
        assert_eq!(result.get(&b"a"[..]).unwrap(), b"2");

        // Range before any snapshot
        assert!(engine.query_range(0, ts1 - 1).is_none());
    }

    #[test]
    fn test_snapshot_eviction() {
        let mut engine = TimeTravelEngine::new(2);

        engine.capture(make_data(&[(b"a", b"1")]), "snap1");
        engine.capture(make_data(&[(b"b", b"2")]), "snap2");
        engine.capture(make_data(&[(b"c", b"3")]), "snap3");

        assert_eq!(engine.snapshot_count(), 2);
    }

    #[test]
    fn test_list_and_remove_snapshots() {
        let mut engine = TimeTravelEngine::new(10);

        engine.capture(make_data(&[(b"x", b"1")]), "first");
        engine.capture(make_data(&[(b"y", b"2")]), "second");

        assert_eq!(engine.snapshot_count(), 2);
        let list = engine.list_snapshots();
        assert_eq!(list.len(), 2);

        let removed = engine.remove_snapshot(list[0].0);
        assert!(removed);
        assert_eq!(engine.snapshot_count(), 1);
    }

    #[test]
    fn test_clear() {
        let mut engine = TimeTravelEngine::new(10);
        engine.capture(make_data(&[(b"a", b"1")]), "snap");
        engine.clear();
        assert_eq!(engine.snapshot_count(), 0);
    }

    // ── Persistence tests ─────────────────────────────────────────────────

    #[test]
    fn test_persist_and_restore_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let mut engine = TimeTravelEngine::new_with_persistence(10, base.clone());
        let data = make_data(&[(b"key1", b"val1"), (b"key2", b"val2")]);
        let ts = engine.capture(data, "test-snap");

        // Verify the file exists on disk
        let filename = format!("snapshot_{}.json", ts);
        let file_path = base.join(&filename);
        assert!(file_path.exists(), "Snapshot file should exist on disk");

        // Drop the engine (simulating restart)
        drop(engine);

        // Create a new engine pointing to the same path
        let mut restored_engine = TimeTravelEngine::new_with_persistence(10, base.clone());

        // Initially no snapshots in memory
        assert_eq!(restored_engine.snapshot_count(), 0);

        // Restore from disk
        let count = restored_engine.restore_snapshots().unwrap();
        assert_eq!(count, 1, "Should restore 1 snapshot");
        assert_eq!(restored_engine.snapshot_count(), 1);

        // Verify the snapshot data is accessible
        assert_eq!(
            restored_engine.query_as_of(b"key1", ts + 1),
            Some(b"val1".to_vec())
        );
        assert_eq!(
            restored_engine.query_as_of(b"key2", ts + 1),
            Some(b"val2".to_vec())
        );

        let snapshots = restored_engine.list_snapshots();
        assert_eq!(snapshots[0].1, "test-snap");
    }

    #[test]
    fn test_persist_multiple_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let mut engine = TimeTravelEngine::new_with_persistence(10, base.clone());

        let ts1 = engine.capture(make_data(&[(b"a", b"1")]), "first");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let ts2 = engine.capture(make_data(&[(b"b", b"2")]), "second");

        // Both files should exist
        assert!(base.join(format!("snapshot_{}.json", ts1)).exists());
        assert!(base.join(format!("snapshot_{}.json", ts2)).exists());

        // Clear memory and restore
        engine.clear();
        assert_eq!(engine.snapshot_count(), 0);

        let count = engine.restore_snapshots().unwrap();
        assert_eq!(count, 2);
        assert_eq!(engine.snapshot_count(), 2);
    }

    #[test]
    fn test_persist_and_restore_empty_base_path() {
        let mut engine = TimeTravelEngine::new(5);
        engine.capture(make_data(&[(b"a", b"1")]), "snap");

        // Without persistence, load_snapshots should return error
        let result = engine.load_snapshots();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Persistence not configured"));
    }

    #[test]
    fn test_restore_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let mut engine = TimeTravelEngine::new_with_persistence(10, base);
        let count = engine.restore_snapshots().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_persistence_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        // Engine with max_snapshots=2
        let mut engine = TimeTravelEngine::new_with_persistence(2, base.clone());

        engine.capture(make_data(&[(b"a", b"1")]), "snap1");
        engine.capture(make_data(&[(b"b", b"2")]), "snap2");
        let ts3 = engine.capture(make_data(&[(b"c", b"3")]), "snap3");

        // Memory should have only 2 snapshots (snap3 and snap2, snap1 evicted)
        assert_eq!(engine.snapshot_count(), 2);

        // All 3 files should still be on disk (persistence doesn't delete files)
        let entries = engine.load_snapshots().unwrap();
        assert_eq!(entries.len(), 3, "All 3 files should still exist on disk");

        // But only snap2 and snap3 can be queried (snap1 evicted from memory)
        assert!(engine.query_as_of(b"c", ts3 + 1).is_some());
    }

    #[test]
    fn test_persist_snapshot_method() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let engine = TimeTravelEngine::new_with_persistence(10, base.clone());

        let snapshot = Snapshot {
            timestamp: 42,
            data: make_data(&[(b"k", b"v")]),
            label: "manual".to_string(),
        };

        engine.persist_snapshot(&snapshot).unwrap();

        let file_path = base.join("snapshot_42.json");
        assert!(file_path.exists());

        // Read back and verify
        let raw = fs::read_to_string(&file_path).unwrap();
        let restored: Snapshot = serde_json::from_str(&raw).unwrap();
        assert_eq!(restored.timestamp, 42);
        assert_eq!(restored.label, "manual");
        assert_eq!(restored.data.get(&b"k".to_vec()).unwrap(), &b"v".to_vec());
    }

    #[test]
    fn test_persist_snapshot_no_base_path() {
        let engine = TimeTravelEngine::new(5);
        let snapshot = Snapshot {
            timestamp: 1,
            data: HashMap::new(),
            label: "test".to_string(),
        };
        let result = engine.persist_snapshot(&snapshot);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Persistence not configured"));
    }

    #[test]
    fn test_base_path_accessor() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_path_buf();

        let engine = TimeTravelEngine::new(5);
        assert!(engine.base_path().is_none());

        let engine2 = TimeTravelEngine::new_with_persistence(10, base.clone());
        assert_eq!(engine2.base_path().unwrap(), &base);
    }
}
