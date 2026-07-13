//! Health monitoring — disk, integrity, and process health.
//!
//! This module consolidates three subsystems that monitor the health of the
//! storage engine and its environment:
//!
//! - **`disk_monitor`** — background disk space probing with configurable
//!   thresholds and automatic read-only mode when disk is critically full.
//! - **`scrubber`** — SSTable integrity verification (CRC32, magic bytes) and
//!   orphan file detection for data corruption discovery.
//! - **`watchdog`** — background health monitor that tracks operation latencies
//!   and triggers warnings when user-defined thresholds are exceeded.

pub mod disk_monitor;
pub mod scrubber;
pub mod watchdog;
