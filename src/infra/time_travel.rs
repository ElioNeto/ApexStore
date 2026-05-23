//! Time-travel queries — query the store as it appeared at a past point in time.
//!
//! [`TimeTravelEngine`] keeps historical snapshots (key-value pairs annotated
//! with timestamps) and allows querying the data as it existed at a given
//! moment or within a time window.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A snapshot of engine state captured at a specific instant.
#[derive(Debug, Clone)]
struct Snapshot {
    /// Monotonic timestamp (nanoseconds since Unix epoch).
    timestamp: u128,
    /// All key-value pairs at that moment.
    data: HashMap<Vec<u8>, Vec<u8>>,
    /// Human-readable label for the snapshot.
    label: String,
}

/// Engine for time-travel queries.
///
/// Snapshots are stored in memory.  Each snapshot captures the full state
/// of a column family at a given timestamp.  Queries return the data as it
/// existed at or before the requested time point.
pub struct TimeTravelEngine {
    /// All captured snapshots, sorted by timestamp (oldest first).
    snapshots: Vec<Snapshot>,
    /// Maximum number of snapshots to retain.
    max_snapshots: usize,
}

impl TimeTravelEngine {
    /// Create a new time-travel engine with the given capacity.
    ///
    /// `max_snapshots` limits how many historical snapshots are kept.
    /// When the limit is exceeded, the oldest snapshots are evicted.
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
        }
    }

    /// Capture the current engine state as a snapshot.
    ///
    /// `data` should be a full dump of the column family at this instant.
    /// `label` is an optional human-readable name for the snapshot.
    pub fn capture(&mut self, data: HashMap<Vec<u8>, Vec<u8>>, label: &str) -> u128 {
        let timestamp = now_nanos();

        self.snapshots.push(Snapshot {
            timestamp,
            data,
            label: label.to_string(),
        });

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

    /// Clear all snapshots.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(pairs: &[(&[u8], &[u8])]) -> HashMap<Vec<u8>, Vec<u8>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect()
    }

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
}
