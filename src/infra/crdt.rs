//! CRDT-based real-time collaboration types.
//!
//! This module provides:
//!
//! - [`CrdtEngine`] — a simple last-writer-wins CRDT engine that tracks
//!   key-value pairs with associated timestamps and can resolve conflicts.
//! - [`CrdtEntry`] — a single entry with key, value, and timestamp.
//! - [`GCounter`] — a grow-only counter (increment-only).
//! - [`PNCounter`] — a positive-negative counter (supports increment and decrement).
//! - [`ORSet`] — an observed-remove set based on unique tags.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

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

    /// Return all entries currently tracked by the CRDT engine.
    pub fn get_all_entries(&self) -> Vec<CrdtEntry> {
        self.state
            .iter()
            .map(|(key, (value, timestamp))| CrdtEntry {
                key: key.clone(),
                value: value.clone(),
                timestamp: *timestamp,
            })
            .collect()
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

// ── G-Counter (Grow-only Counter) ────────────────────────────────────────────

/// A grow-only counter CRDT.
///
/// Each node holds its own count, and the total value is the sum of all
/// node counts. Merge takes the maximum per node.
#[derive(Debug, Clone)]
pub struct GCounter {
    counters: HashMap<String, u64>,
}

impl GCounter {
    /// Create a new empty `GCounter`.
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    /// Increment the counter for a given node by `amount`.
    pub fn increment(&mut self, node_id: &str, amount: u64) {
        let entry = self.counters.entry(node_id.to_string()).or_insert(0);
        *entry = entry.saturating_add(amount);
    }

    /// Return the total value (sum of all node counters).
    pub fn value(&self) -> u64 {
        self.counters.values().sum()
    }

    /// Merge another `GCounter` into this one, taking the max per node.
    pub fn merge(&mut self, other: &GCounter) {
        for (node, &count) in &other.counters {
            let entry = self.counters.entry(node.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }
}

impl Default for GCounter {
    fn default() -> Self {
        Self::new()
    }
}

// ── PN-Counter (Positive-Negative Counter) ──────────────────────────────────

/// A positive-negative counter CRDT.
///
/// Supports both increment and decrement operations. Internally uses two
/// [`GCounter`]s: one for positive increments and one for negative decrements.
/// The final value is `pos - neg`.
#[derive(Debug, Clone)]
pub struct PNCounter {
    positive: GCounter,
    negative: GCounter,
}

impl PNCounter {
    /// Create a new `PNCounter` with zero value.
    pub fn new() -> Self {
        Self {
            positive: GCounter::new(),
            negative: GCounter::new(),
        }
    }

    /// Increment the counter for a given node by `amount`.
    pub fn increment(&mut self, node_id: &str, amount: u64) {
        self.positive.increment(node_id, amount);
    }

    /// Decrement the counter for a given node by `amount`.
    pub fn decrement(&mut self, node_id: &str, amount: u64) {
        self.negative.increment(node_id, amount);
    }

    /// Return the current value (positive total minus negative total).
    pub fn value(&self) -> i64 {
        (self.positive.value() as i64) - (self.negative.value() as i64)
    }

    /// Merge another `PNCounter` into this one, merging both internal counters.
    pub fn merge(&mut self, other: &PNCounter) {
        self.positive.merge(&other.positive);
        self.negative.merge(&other.negative);
    }
}

impl Default for PNCounter {
    fn default() -> Self {
        Self::new()
    }
}

// ── OR-Set (Observed-Remove Set) ────────────────────────────────────────────

/// An observed-remove set CRDT.
///
/// Each element is associated with a set of unique tags. Adding an element
/// inserts a new tag; removing an element clears all of its tags. An element
/// is considered present in the set only when it has at least one tag.
#[derive(Debug, Clone)]
pub struct ORSet<T: Hash + Eq + Clone> {
    elements: HashMap<T, HashSet<String>>,
}

impl<T: Hash + Eq + Clone> ORSet<T> {
    /// Create a new empty `ORSet`.
    pub fn new() -> Self {
        Self {
            elements: HashMap::new(),
        }
    }

    /// Add an element with a unique tag.
    ///
    /// If the element was previously removed, this re-adds it under a new tag.
    pub fn add(&mut self, element: T, tag: String) {
        self.elements.entry(element).or_default().insert(tag);
    }

    /// Remove an element by clearing all of its tags.
    ///
    /// After this, `contains()` returns `false` for the element.
    pub fn remove(&mut self, element: &T) {
        self.elements.remove(element);
    }

    /// Check whether the set contains an element.
    ///
    /// An element is present if it has one or more associated tags.
    pub fn contains(&self, element: &T) -> bool {
        self.elements
            .get(element)
            .map(|tags| !tags.is_empty())
            .unwrap_or(false)
    }

    /// Return all elements currently in the set.
    pub fn elements(&self) -> Vec<&T> {
        self.elements
            .iter()
            .filter(|(_, tags)| !tags.is_empty())
            .map(|(elem, _)| elem)
            .collect()
    }

    /// Merge another `ORSet` into this one, taking the union of tag sets.
    pub fn merge(&mut self, other: &ORSet<T>) {
        for (elem, tags) in &other.elements {
            let entry = self.elements.entry(elem.clone()).or_default();
            entry.extend(tags.iter().cloned());
        }
    }
}

impl<T: Hash + Eq + Clone> Default for ORSet<T> {
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
        assert_eq!(engine.get_state(b"key1"), Some((b"value1".to_vec(), 100)));
    }

    #[test]
    fn test_merge_update_newer() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"key1".to_vec(), b"value1".to_vec(), 100);
        engine.merge(b"key1".to_vec(), b"value2".to_vec(), 200);
        assert_eq!(engine.get_state(b"key1"), Some((b"value2".to_vec(), 200)));
    }

    #[test]
    fn test_merge_older_ignored() {
        let mut engine = CrdtEngine::new();
        engine.merge(b"key1".to_vec(), b"newer".to_vec(), 200);
        engine.merge(b"key1".to_vec(), b"older".to_vec(), 100);
        // The older timestamp should be ignored.
        assert_eq!(engine.get_state(b"key1"), Some((b"newer".to_vec(), 200)));
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

    // ── GCounter tests ─────────────────────────────────────────────────────

    #[test]
    fn test_gcounter_new_value_zero() {
        let counter = GCounter::new();
        assert_eq!(counter.value(), 0);
    }

    #[test]
    fn test_gcounter_increment() {
        let mut counter = GCounter::new();
        counter.increment("node1", 5);
        counter.increment("node2", 3);
        assert_eq!(counter.value(), 8);
    }

    #[test]
    fn test_gcounter_merge_takes_max() {
        let mut a = GCounter::new();
        a.increment("node1", 10);
        a.increment("node2", 5);

        let mut b = GCounter::new();
        b.increment("node1", 3);
        b.increment("node2", 8);
        b.increment("node3", 2);

        a.merge(&b);
        assert_eq!(a.value(), 20); // 10 (max node1) + 8 (max node2) + 2 (node3)
    }

    #[test]
    fn test_gcounter_saturating_add() {
        let mut counter = GCounter::new();
        counter.increment("node1", u64::MAX);
        counter.increment("node1", 1);
        assert_eq!(counter.value(), u64::MAX);
    }

    // ── PNCounter tests ────────────────────────────────────────────────────

    #[test]
    fn test_pncounter_new_value_zero() {
        let counter = PNCounter::new();
        assert_eq!(counter.value(), 0);
    }

    #[test]
    fn test_pncounter_increment_decrement() {
        let mut counter = PNCounter::new();
        counter.increment("node1", 10);
        counter.decrement("node1", 3);
        assert_eq!(counter.value(), 7);
    }

    #[test]
    fn test_pncounter_negative_value() {
        let mut counter = PNCounter::new();
        counter.decrement("node1", 5);
        assert_eq!(counter.value(), -5);
    }

    #[test]
    fn test_pncounter_merge() {
        let mut a = PNCounter::new();
        a.increment("n1", 10);
        a.decrement("n1", 2);

        let mut b = PNCounter::new();
        b.increment("n1", 5);
        b.decrement("n1", 1);
        b.increment("n2", 3);

        a.merge(&b);
        // pos: max(10,5)=10 + n2=3 = 13
        // neg: max(2,1)=2
        assert_eq!(a.value(), 11);
    }

    // ── ORSet tests ────────────────────────────────────────────────────────

    #[test]
    fn test_orset_add_and_contains() {
        let mut set: ORSet<&str> = ORSet::new();
        set.add("apple", "tag1".to_string());
        assert!(set.contains(&"apple"));
        assert!(!set.contains(&"banana"));
    }

    #[test]
    fn test_orset_remove() {
        let mut set: ORSet<String> = ORSet::new();
        set.add("apple".to_string(), "tag1".to_string());
        set.remove(&"apple".to_string());
        assert!(!set.contains(&"apple".to_string()));
    }

    #[test]
    fn test_orset_readd_after_remove() {
        let mut set: ORSet<String> = ORSet::new();
        set.add("x".to_string(), "tag1".to_string());
        set.remove(&"x".to_string());
        set.add("x".to_string(), "tag2".to_string());
        assert!(set.contains(&"x".to_string()));
    }

    #[test]
    fn test_orset_elements() {
        let mut set: ORSet<&str> = ORSet::new();
        set.add("a", "t1".to_string());
        set.add("b", "t2".to_string());
        set.add("c", "t3".to_string());
        set.remove(&"b");
        let elems = set.elements();
        assert_eq!(elems.len(), 2);
        assert!(elems.contains(&&"a"));
        assert!(elems.contains(&&"c"));
    }

    #[test]
    fn test_orset_merge() {
        let mut a: ORSet<&str> = ORSet::new();
        a.add("x", "t1".to_string());
        a.add("y", "t2".to_string());

        let mut b: ORSet<&str> = ORSet::new();
        b.add("y", "t3".to_string());
        b.add("z", "t4".to_string());

        a.merge(&b);
        assert!(a.contains(&"x"));
        assert!(a.contains(&"y"));
        assert!(a.contains(&"z"));
        assert_eq!(a.elements().len(), 3);
    }

    #[test]
    fn test_orset_empty() {
        let set: ORSet<String> = ORSet::new();
        assert!(set.elements().is_empty());
        assert!(!set.contains(&"anything".to_string()));
    }
}
