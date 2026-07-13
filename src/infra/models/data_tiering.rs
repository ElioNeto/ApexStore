//! Automatic data tiering — manage hot/warm/cold data placement.
//!
//! [`DataTieringConfig`] tracks which storage tier a key belongs to and
//! provides methods for promoting and demoting data between tiers.
//!
//! # Tiers
//!
//! - **Hot** — frequently accessed data, kept in memory (memtable / block cache).
//! - **Warm** — recently accessed data on fast local storage (NVMe / SSD).
//! - **Cold** — infrequently accessed data on cheaper storage (HDD / object store).
//!
//! # Status
//!
//! This module has a working metadata-tracking layer with auto-promotion, ageing,
//! and compaction hints. A production-grade implementation would additionally
//! integrate with the storage engine's compaction policy and block cache to
//! physically move data between storage tiers.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The storage tier for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Hot data — kept in memory.
    Hot,
    /// Warm data — on fast local storage.
    Warm,
    /// Cold data — on cheap/archival storage.
    Cold,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Hot => write!(f, "hot"),
            Tier::Warm => write!(f, "warm"),
            Tier::Cold => write!(f, "cold"),
        }
    }
}

/// Metadata for a key's tier placement.
#[derive(Debug, Clone)]
struct TierEntry {
    tier: Tier,
    /// Last access timestamp (nanoseconds since Unix epoch).
    last_access: u128,
    /// Access frequency counter.
    access_count: u64,
}

/// Configuration and state for automatic data tiering.
///
/// Tracks per-key tier assignments and provides methods to promote
/// (move to a faster tier) or demote (move to a slower tier) data.
///
/// # Note
///
/// The metadata-tracking layer (tier assignments, auto-promotion, ageing) is
/// fully implemented. A production-grade extension would integrate with
/// the storage engine's compaction policy and block cache to physically
/// move data between storage tiers.
pub struct DataTieringConfig {
    /// Per-key tier metadata.
    entries: HashMap<Vec<u8>, TierEntry>,
    /// Access threshold (count) before promoting to Hot.
    hot_threshold: u64,
    /// Age threshold (seconds) before demoting to Cold.
    cold_age_secs: u64,
    /// Current default tier for new keys.
    default_tier: Tier,
}

impl DataTieringConfig {
    /// Create a new data tiering config with the given thresholds.
    ///
    /// * `hot_threshold` — number of accesses before a key is promoted to Hot.
    /// * `cold_age_secs` — seconds of inactivity before a key is demoted to Cold.
    pub fn new(hot_threshold: u64, cold_age_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            hot_threshold,
            cold_age_secs,
            default_tier: Tier::Warm,
        }
    }

    /// Record an access to `key` and optionally promote/demote.
    ///
    /// This is called internally by `get_tier()` to keep access statistics.
    fn record_access(&mut self, key: &[u8]) {
        let now = now_nanos();
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_access = now;
            entry.access_count = entry.access_count.saturating_add(1);

            // Auto-promote if hot threshold reached and currently Warm.
            if entry.access_count >= self.hot_threshold && entry.tier == Tier::Warm {
                entry.tier = Tier::Hot;
            }
        }
    }

    /// Manually promote a key to the Hot tier.
    ///
    /// Returns `Ok(())` if the key exists and was promoted, or an error
    /// if the key is not tracked.
    pub fn promote(&mut self, key: &[u8]) -> Result<(), String> {
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.tier = Tier::Hot;
                Ok(())
            }
            None => Err(format!(
                "key {:?} is not tracked for tiering",
                String::from_utf8_lossy(key)
            )),
        }
    }

    /// Manually demote a key to the Cold tier.
    ///
    /// Returns `Ok(())` if the key exists and was demoted, or an error
    /// if the key is not tracked.
    pub fn demote(&mut self, key: &[u8]) -> Result<(), String> {
        match self.entries.get_mut(key) {
            Some(entry) => {
                entry.tier = Tier::Cold;
                Ok(())
            }
            None => Err(format!(
                "key {:?} is not tracked for tiering",
                String::from_utf8_lossy(key)
            )),
        }
    }

    /// Get the current tier for a key.
    ///
    /// Records an access to this key (for auto-promotion logic).
    /// If the key is not yet tracked, it is added with the default tier.
    pub fn get_tier(&mut self, key: &[u8]) -> Tier {
        if !self.entries.contains_key(key) {
            self.entries.insert(
                key.to_vec(),
                TierEntry {
                    tier: self.default_tier,
                    last_access: now_nanos(),
                    access_count: 0,
                },
            );
            return self.default_tier;
        }

        self.record_access(key);
        self.entries[key].tier
    }

    /// Set the default tier for new keys.
    pub fn set_default_tier(&mut self, tier: Tier) {
        self.default_tier = tier;
    }

    /// Return the default tier.
    pub fn default_tier(&self) -> Tier {
        self.default_tier
    }

    /// Run a maintenance pass: demote old Hot/Warm keys to Cold.
    ///
    /// Should be called periodically (e.g. every 60 seconds).
    pub fn age_out(&mut self) {
        let now = now_nanos();
        let cold_age_ns = Duration::from_secs(self.cold_age_secs).as_nanos();

        for entry in self.entries.values_mut() {
            if entry.tier != Tier::Cold && now.saturating_sub(entry.last_access) > cold_age_ns {
                entry.tier = Tier::Cold;
            }
        }
    }

    /// Stop tracking a key.
    pub fn forget(&mut self, key: &[u8]) {
        self.entries.remove(key);
    }

    /// Return the number of tracked keys.
    pub fn tracked_keys(&self) -> usize {
        self.entries.len()
    }

    /// Return a breakdown of keys by tier.
    pub fn tier_counts(&self) -> std::collections::BTreeMap<Tier, usize> {
        let mut counts = std::collections::BTreeMap::new();
        for entry in self.entries.values() {
            *counts.entry(entry.tier).or_insert(0) += 1;
        }
        counts
    }

    /// Determine whether data in the given tier should be compacted.
    ///
    /// Returns `true` for Hot and Warm tiers (data that benefits from
    /// compaction), and `false` for Cold (archival) data.
    pub fn should_compact_to_tier(tier: Tier) -> bool {
        matches!(tier, Tier::Hot | Tier::Warm)
    }

    /// Return all keys currently assigned to a specific tier.
    ///
    /// This is useful for compaction policies that need to act on all keys
    /// in a given tier.
    pub fn keys_for_tier(&self, tier: Tier) -> Vec<Vec<u8>> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.tier == tier)
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Look up the tier for a key without recording an access.
    ///
    /// Returns `None` if the key is not tracked.
    pub fn get_tier_for_key(&self, key: &[u8]) -> Option<Tier> {
        self.entries.get(key).map(|entry| entry.tier)
    }
}

/// Hint for the compaction system about which tier and keys to compact.
#[derive(Debug, Clone)]
pub struct CompactionHint {
    /// The storage tier to compact.
    pub tier: Tier,
    /// The keys that should be considered for compaction in this tier.
    pub keys: Vec<Vec<u8>>,
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

    #[test]
    fn test_default_tier() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        assert_eq!(cfg.get_tier(b"new_key"), Tier::Warm);
        assert_eq!(cfg.tracked_keys(), 1);
    }

    #[test]
    fn test_promote_and_demote() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        cfg.get_tier(b"my_key"); // tracks the key as Warm

        cfg.promote(b"my_key").unwrap();
        assert_eq!(cfg.get_tier(b"my_key"), Tier::Hot);

        cfg.demote(b"my_key").unwrap();
        assert_eq!(cfg.get_tier(b"my_key"), Tier::Cold);
    }

    #[test]
    fn test_promote_untracked_key() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        let result = cfg.promote(b"nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_auto_promote_on_access() {
        let mut cfg = DataTieringConfig::new(3, 3600); // promote after 3 accesses
        cfg.get_tier(b"k"); // access 1 — Warm

        cfg.get_tier(b"k"); // access 2 — still Warm
        assert_eq!(cfg.get_tier(b"k"), Tier::Warm);

        cfg.get_tier(b"k"); // access 3 — should be Hot now
        assert_eq!(cfg.get_tier(b"k"), Tier::Hot);
    }

    #[test]
    fn test_age_out() {
        let mut cfg = DataTieringConfig::new(5, 0); // age out immediately (0 sec)
        cfg.get_tier(b"k"); // Warm
        cfg.age_out(); // should demote to Cold
        assert_eq!(cfg.get_tier(b"k"), Tier::Cold);
    }

    #[test]
    fn test_forget() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        cfg.get_tier(b"k");
        assert_eq!(cfg.tracked_keys(), 1);
        cfg.forget(b"k");
        assert_eq!(cfg.tracked_keys(), 0);
    }

    #[test]
    fn test_tier_counts() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        cfg.get_tier(b"a");
        cfg.get_tier(b"b");
        cfg.promote(b"a").unwrap();

        let counts = cfg.tier_counts();
        assert_eq!(*counts.get(&Tier::Hot).unwrap_or(&0), 1);
        assert_eq!(*counts.get(&Tier::Warm).unwrap_or(&0), 1);
    }

    #[test]
    fn test_display_tier() {
        assert_eq!(format!("{}", Tier::Hot), "hot");
        assert_eq!(format!("{}", Tier::Warm), "warm");
        assert_eq!(format!("{}", Tier::Cold), "cold");
    }

    // ── Integration method tests ───────────────────────────────────────────

    #[test]
    fn test_should_compact_to_tier() {
        assert!(DataTieringConfig::should_compact_to_tier(Tier::Hot));
        assert!(DataTieringConfig::should_compact_to_tier(Tier::Warm));
        assert!(!DataTieringConfig::should_compact_to_tier(Tier::Cold));
    }

    #[test]
    fn test_keys_for_tier() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        cfg.get_tier(b"hot_key");
        cfg.get_tier(b"warm_key");
        cfg.get_tier(b"other_warm");
        cfg.promote(b"hot_key").unwrap();

        let hot_keys = cfg.keys_for_tier(Tier::Hot);
        assert_eq!(hot_keys.len(), 1);
        assert_eq!(hot_keys[0], b"hot_key");

        let warm_keys = cfg.keys_for_tier(Tier::Warm);
        assert_eq!(warm_keys.len(), 2);
    }

    #[test]
    fn test_keys_for_tier_empty() {
        let cfg = DataTieringConfig::new(5, 3600);
        let hot_keys = cfg.keys_for_tier(Tier::Hot);
        assert!(hot_keys.is_empty());
    }

    #[test]
    fn test_get_tier_for_key() {
        let mut cfg = DataTieringConfig::new(5, 3600);
        cfg.get_tier(b"my_key");
        assert_eq!(cfg.get_tier_for_key(b"my_key"), Some(Tier::Warm));

        cfg.promote(b"my_key").unwrap();
        assert_eq!(cfg.get_tier_for_key(b"my_key"), Some(Tier::Hot));
    }

    #[test]
    fn test_get_tier_for_key_untracked() {
        let cfg = DataTieringConfig::new(5, 3600);
        assert_eq!(cfg.get_tier_for_key(b"unknown"), None);
    }

    #[test]
    fn test_compaction_hint_struct() {
        let hint = CompactionHint {
            tier: Tier::Hot,
            keys: vec![b"k1".to_vec(), b"k2".to_vec()],
        };
        assert_eq!(hint.tier, Tier::Hot);
        assert_eq!(hint.keys.len(), 2);
    }
}
