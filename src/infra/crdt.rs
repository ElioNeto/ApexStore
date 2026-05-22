//! CRDT-based real-time collaboration — LWW (Last-Writer-Wins) register.
//!
//! This module provides:
//!
//! - [`CrdtEngine`] — a simple last-writer-wins CRDT engine that tracks
//!   key-value pairs with associated timestamps and can resolve conflicts.
//! - [`CrdtEntry`] — a single entry with key, value, and timestamp.

use std::collections::HashMap;

/// A single CRDT entry with its assigned timestamp.
#[derive(Debug, Clone, PartialEq)]
pub struct CrdtEntry {
    /// The key (binary).
    pub key: Vec<u8>,
    /// The value (binary).
    pub value: Vec<u8>,
    /// Monotonic timestamp used for conflict resolution (higher wins).
    pub timestamp: u64,
}

/// A Last-Writer-Wins (LWW) CRDT engine.
///
/// Internally stores a map of key → (value, timestamp). When merging,
/// the entry with the highest timestamp wins.
pub struct CrdtEngine {
    state: HashMap<Vec<u8>, (Vec<u8>, u64)>,
}

impl CrdtEngine {
    /// Create a new empty CRDT engine.
    pub fn new() -> Self {
        Self {
            state: HashMap::new(),
        }
    }

    /// Merge a key-value pair with the given timestamp.
    ///
    /// If the key already exists, the entry with the higher timestamp wins.
    pub fn merge(&mut self, key: Vec<u8>, value: Vec<u8>, timestamp: u64) {
        match self.state.get(&key) {
            Some((_, existing_ts)) if *existing_ts >= timestamp => {
                // Existing entry is newer or equal; keep it.
            }
            _ => {
                self.state.insert(key, (value, timestamp));
            }
        }
    }

    /// Resolve conflicts for a key by returning the entry with the highest
    /// timestamp. If the key does not exist, returns `None`.
    pub fn resolve_conflicts(&self, key: &[u8]) -> Option<CrdtEntry> {
        self.state.get(key).map(|(value, ts)| CrdtEntry {
            key: key.to_vec(),
            value: value.clone(),
            timestamp: *ts,
        })
    }

    /// Return the current state (value and timestamp) for a key, if present.
    pub fn get_state(&self, key: &[u8]) -> Option<(Vec<u8>, u64)> {
        self.state.get(key).cloned()
    }

    /// Return the number of entries tracked.
    pub fn len(&self) -> usize {
        self.state.len()
    }

    /// Returns `true` if the engine has no entries.
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// Clear all tracked state.
    pub fn clear(&mut self) {
        self.state.clear();
    }
}

impl Default for CrdtEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_new_key() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"key1".to_vec(), b"value1".to_vec(), 100);
        assert_eq!(engine.len(), 1);
        assert_eq!(
            engine.get_state(b"key1"),
            Some((b"value1".to_vec(), 100))
        );
    }

    #[test]
    fn test_merge_update_newer() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"key1".to_vec(), b"value1".to_vec(), 100);
        engine.merge(b"key1".to_vec(), b"value2".to_vec(), 200);
        assert_eq!(
            engine.get_state(b"key1"),
            Some((b"value2".to_vec(), 200))
        );
    }

    #[test]
    fn test_merge_older_ignored() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"key1".to_vec(), b"newer".to_vec(), 200);
        engine.merge(b"key1".to_vec(), b"older".to_vec(), 100);
        // The older timestamp should be ignored.
        assert_eq!(
            engine.get_state(b"key1"),
            Some((b"newer".to_vec(), 200))
        );
    }

    #[test]
    fn test_resolve_conflicts() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"a".to_vec(), b"v1".to_vec(), 10);
        engine.merge(b"a".to_vec(), b"v2".to_vec(), 20);
        let entry = engine.resolve_conflicts(b"a").unwrap();
        assert_eq!(entry.value, b"v2".to_vec());
        assert_eq!(entry.timestamp, 20);
    }

    #[test]
    fn test_resolve_conflicts_missing() {
        let engine = CrdtEngine::new();
        assert!(engine.resolve_conflicts(b"nonexistent").is_none());
    }

    #[test]
    fn test_clear() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"k".to_vec(), b"v".to_vec(), 1);
        engine.clear();
        assert!(engine.is_empty());
    }
}
