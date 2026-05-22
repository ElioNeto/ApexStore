use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Represents a single key-value record in the LSM-tree.
///
/// Can represent either a live value, a point tombstone (deleted key),
/// or a range tombstone (deleted key range).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRecord {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp: u128,
    pub is_deleted: bool,
    #[serde(default)]
    pub column_family: Option<String>,
    /// Timestamp (in nanos since UNIX_EPOCH) when this key expires.
    /// `None` means the key never expires.
    #[serde(default)]
    pub expires_at: Option<u128>,
    /// When set, this record is a range tombstone covering [range_start, range_end).
    /// For range tombstones, `key` is set to `range_start` and `is_deleted` is true.
    #[serde(default)]
    pub range_start: Option<Vec<u8>>,
    /// End of the range tombstone (exclusive).
    #[serde(default)]
    pub range_end: Option<Vec<u8>>,
}

impl LogRecord {
    pub fn new(key: Vec<u8>, value: Vec<u8>) -> Self {
        Self {
            key,
            value,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            is_deleted: false,
            column_family: None,
            expires_at: None,
            range_start: None,
            range_end: None,
        }
    }

    pub fn tombstone(key: Vec<u8>) -> Self {
        Self {
            key,
            value: Vec::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            is_deleted: true,
            column_family: None,
            expires_at: None,
            range_start: None,
            range_end: None,
        }
    }

    /// Create a new record with a Time-To-Live (TTL).
    ///
    /// The key will be considered expired after `ttl` duration from now.
    /// `expires_at` is set to `current_time + ttl` in nanos.
    pub fn new_with_ttl(key: Vec<u8>, value: Vec<u8>, ttl: std::time::Duration) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            key,
            value,
            timestamp: now,
            is_deleted: false,
            column_family: None,
            expires_at: Some(now.saturating_add(ttl.as_nanos())),
            range_start: None,
            range_end: None,
        }
    }

    /// Returns `true` if this record has expired relative to the given `now` timestamp (in nanos).
    pub fn is_expired_at(&self, now: u128) -> bool {
        self.expires_at.map_or(false, |exp| now >= exp)
    }

    /// Returns `true` if this record has expired relative to the current system time.
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.is_expired_at(now)
    }

    /// Create a range tombstone record that covers [start, end).
    pub fn range_tombstone(start: Vec<u8>, end: Vec<u8>) -> Self {
        Self {
            key: start.clone(),
            value: Vec::new(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            is_deleted: true,
            column_family: None,
            expires_at: None,
            range_start: Some(start),
            range_end: Some(end),
        }
    }

    /// Returns true if this record is a range tombstone.
    pub fn is_range_tombstone(&self) -> bool {
        self.range_start.is_some() && self.range_end.is_some()
    }
}

/// Represents a range of deleted keys `[start_key, end_key)`.
///
/// Used by the compaction layer and memtable to track range tombstones
/// that have been flushed but are still in effect for ongoing reads.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangeTombstone {
    pub start_key: Vec<u8>,
    pub end_key: Vec<u8>,
    pub timestamp: u128,
}

impl RangeTombstone {
    /// Create a new range tombstone.
    pub fn new(start_key: Vec<u8>, end_key: Vec<u8>) -> Self {
        Self {
            start_key,
            end_key,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        }
    }

    /// Returns `true` if `key` falls within `[start_key, end_key)`.
    pub fn covers(&self, key: &[u8]) -> bool {
        key >= self.start_key.as_slice() && key < self.end_key.as_slice()
    }
}
