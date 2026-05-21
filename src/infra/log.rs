//! Structured usage logging for ApexStore.
//!
//! Provides a lightweight operation log that records all engine operations
//! (set, get, delete, scan, etc.) with timestamps, severity levels, and
//! metadata such as key names, durations, and result sizes.
//!
//! # Consumption
//!
//! - The **TUI** renders the log via [`UsageLog::entries`] as a scrolling list.
//! - The **server** feeds [`tracing`] events to the log when the `tracing`
//!   subscriber is configured.
//!
//! The log is an in-memory ring buffer (configurable capacity, default 1000).
//! It is **not** persisted to disk by default, but entries can be exported
//! via [`UsageLog::export_json`].

use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// Log level
// ---------------------------------------------------------------------------

/// Severity of a usage log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LogLevel {
    /// Debug / detailed diagnostic (not shown in TUI by default).
    Debug,
    /// Normal operational message (e.g. "GET key=foo → 42 bytes").
    Info,
    /// Successful completion of a user-visible operation.
    Success,
    /// Warning — operation succeeded but with caveats.
    Warn,
    /// Error — operation failed.
    Error,
}

impl LogLevel {
    /// Return a short label for display.
    pub fn label(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DBG",
            LogLevel::Info => "INF",
            LogLevel::Success => "OK",
            LogLevel::Warn => "WRN",
            LogLevel::Error => "ERR",
        }
    }
}

// ---------------------------------------------------------------------------
// Log entry
// ---------------------------------------------------------------------------

/// A single entry in the usage log.
#[derive(Debug, Clone, Serialize)]
pub struct UsageEntry {
    /// Wall-clock timestamp when the entry was created.
    pub timestamp: DateTime<Local>,
    /// Severity level.
    pub level: LogLevel,
    /// Human-readable message.
    pub message: String,
    /// Optional duration in milliseconds (for operations like set/get/compact).
    pub duration_ms: Option<f64>,
    /// Optional key involved in the operation.
    pub key: Option<String>,
    /// Optional value size in bytes.
    pub value_size: Option<usize>,
}

impl UsageEntry {
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            timestamp: Local::now(),
            level,
            message: message.into(),
            duration_ms: None,
            key: None,
            value_size: None,
        }
    }

    pub fn with_duration(mut self, ms: f64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_value_size(mut self, size: usize) -> Self {
        self.value_size = Some(size);
        self
    }

    /// Format a short one-line representation for the TUI.
    pub fn format_tui(&self) -> String {
        let ts = self.timestamp.format("%H:%M:%S%.3f");
        let level_flag = self.level.label();
        let duration = self
            .duration_ms
            .map(|d| format!(" [{:.1}ms]", d))
            .unwrap_or_default();
        let key_info = self
            .key
            .as_ref()
            .map(|k| format!(" key={}", k))
            .unwrap_or_default();
        let size_info = self
            .value_size
            .map(|s| format!(" ({} B)", s))
            .unwrap_or_default();

        format!(
            "[{}] {}{}{}{}{}",
            ts, level_flag, duration, key_info, size_info, self.message
        )
    }
}

// ---------------------------------------------------------------------------
// Ring-buffer usage log
// ---------------------------------------------------------------------------

/// In-memory ring buffer of [`UsageEntry`] values.
///
/// # Capacity
///
/// Defaults to 1000 entries.  When full, the oldest entry is popped before
/// pushing a new one.  This avoids unbounded memory growth.
pub struct UsageLog {
    entries: VecDeque<UsageEntry>,
    capacity: usize,
    /// Optional minimum level for filtering (entries below this are dropped).
    min_level: LogLevel,
    /// Cumulative counters by operation type (populated by the engine).
    pub ops_count: u64,
    pub ops_set: u64,
    pub ops_get: u64,
    pub ops_delete: u64,
    pub ops_scan: u64,
    pub ops_other: u64,
}

impl UsageLog {
    /// Create a new usage log with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            min_level: LogLevel::Debug,
            ops_count: 0,
            ops_set: 0,
            ops_get: 0,
            ops_delete: 0,
            ops_scan: 0,
            ops_other: 0,
        }
    }

    /// Push a new entry, evicting the oldest if at capacity.
    pub fn push(&mut self, entry: UsageEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    // ── Convenience builders ────────────────────────────────────────────

    pub fn info(&mut self, msg: impl Into<String>) {
        self.push(UsageEntry::new(LogLevel::Info, msg));
    }

    pub fn success(&mut self, msg: impl Into<String>) {
        self.push(UsageEntry::new(LogLevel::Success, msg));
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.push(UsageEntry::new(LogLevel::Warn, msg));
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.push(UsageEntry::new(LogLevel::Error, msg));
    }

    pub fn debug(&mut self, msg: impl Into<String>) {
        self.push(UsageEntry::new(LogLevel::Debug, msg));
    }

    // ── Operation-logging helpers ───────────────────────────────────────

    /// Log a SET / PUT operation.
    pub fn log_set(&mut self, key: &str, value_size: usize, duration_ms: f64) {
        self.ops_count += 1;
        self.ops_set += 1;
        self.push(
            UsageEntry::new(LogLevel::Success, " SET")
                .with_key(key)
                .with_value_size(value_size)
                .with_duration(duration_ms),
        );
    }

    /// Log a GET operation.
    pub fn log_get(&mut self, key: &str, found: bool, value_size: Option<usize>, duration_ms: f64) {
        self.ops_count += 1;
        self.ops_get += 1;
        if found {
            self.push(
                UsageEntry::new(LogLevel::Success, " GET")
                    .with_key(key)
                    .with_value_size(value_size.unwrap_or(0))
                    .with_duration(duration_ms),
            );
        } else {
            self.push(
                UsageEntry::new(LogLevel::Warn, " GET (not found)")
                    .with_key(key)
                    .with_duration(duration_ms),
            );
        }
    }

    /// Log a DELETE operation.
    pub fn log_delete(&mut self, key: &str, duration_ms: f64) {
        self.ops_count += 1;
        self.ops_delete += 1;
        self.push(
            UsageEntry::new(LogLevel::Success, " DEL")
                .with_key(key)
                .with_duration(duration_ms),
        );
    }

    /// Log a SCAN / SEARCH operation.
    pub fn log_scan(&mut self, prefix: &str, count: usize, duration_ms: f64) {
        self.ops_count += 1;
        self.ops_scan += 1;
        self.push(
            UsageEntry::new(
                LogLevel::Success,
                format!(" SCAN prefix='{}' ({} rows)", prefix, count),
            )
            .with_duration(duration_ms),
        );
    }

    /// Log a COMPACTION event.
    pub fn log_compaction(&mut self, cf: &str, files_merged: usize, duration_ms: f64) {
        self.push(
            UsageEntry::new(
                LogLevel::Info,
                format!(" Compaction (CF={}, files={})", cf, files_merged),
            )
            .with_duration(duration_ms),
        );
    }

    /// Log a FLUSH event.
    pub fn log_flush(&mut self, cf: &str, records: usize, duration_ms: f64) {
        self.push(
            UsageEntry::new(
                LogLevel::Info,
                format!(" Flush (CF={}, {} records)", cf, records),
            )
            .with_duration(duration_ms),
        );
    }

    // ── Queries ─────────────────────────────────────────────────────────

    /// Return all entries (oldest first).
    pub fn entries(&self) -> &VecDeque<UsageEntry> {
        &self.entries
    }

    /// Return entries filtered by minimum level.
    pub fn entries_filtered(&self, min_level: LogLevel) -> Vec<&UsageEntry> {
        let levels = level_rank(min_level);
        self.entries
            .iter()
            .filter(|e| level_rank(e.level) >= levels)
            .collect()
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Set the minimum log level (entries below this are filtered out).
    pub fn set_min_level(&mut self, level: LogLevel) {
        self.min_level = level;
    }

    pub fn get_min_level(&self) -> LogLevel {
        self.min_level
    }

    /// Export all entries as a JSON string (for debugging / external tools).
    pub fn export_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(&self.entries)
    }
}

impl Default for UsageLog {
    fn default() -> Self {
        Self::with_capacity(1000)
    }
}

fn level_rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Debug => 0,
        LogLevel::Info => 1,
        LogLevel::Success => 1,
        LogLevel::Warn => 2,
        LogLevel::Error => 3,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_capacity() {
        let mut log = UsageLog::with_capacity(3);
        log.info("first");
        log.info("second");
        log.info("third");
        assert_eq!(log.len(), 3);

        log.info("fourth"); // evicts "first"
        assert_eq!(log.len(), 3);
        assert!(log.entries[0].message.contains("second"));
    }

    #[test]
    fn test_level_filtering() {
        let mut log = UsageLog::with_capacity(10);
        log.debug("debug msg");
        log.info("info msg");
        log.warn("warn msg");
        log.error("error msg");

        let filtered = log.entries_filtered(LogLevel::Warn);
        assert_eq!(filtered.len(), 2); // warn + error
    }

    #[test]
    fn test_operation_counters() {
        let mut log = UsageLog::with_capacity(10);
        log.log_set("mykey", 42, 1.5);
        assert_eq!(log.ops_set, 1);
        assert_eq!(log.ops_count, 1);

        log.log_get("mykey", true, Some(42), 0.5);
        assert_eq!(log.ops_get, 1);
        assert_eq!(log.ops_count, 2);

        log.log_delete("mykey", 0.3);
        assert_eq!(log.ops_delete, 1);
        assert_eq!(log.ops_count, 3);
    }

    #[test]
    fn test_tui_format() {
        let entry = UsageEntry::new(LogLevel::Success, " SET")
            .with_key("test")
            .with_value_size(100)
            .with_duration(0.42);
        let formatted = entry.format_tui();
        assert!(formatted.contains("OK"));
        assert!(formatted.contains("key=test"));
        assert!(formatted.contains("100 B"));
        assert!(formatted.contains("0.4ms"));
    }

    #[test]
    fn test_clear() {
        let mut log = UsageLog::with_capacity(10);
        log.info("hello");
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn test_json_export() {
        let mut log = UsageLog::with_capacity(10);
        log.info("hello");
        log.warn("world");
        let json = log.export_json().unwrap();
        assert!(json.contains("hello"));
        assert!(json.contains("world"));
        assert!(json.contains("level"));
        assert!(json.contains("timestamp"));
    }
}
