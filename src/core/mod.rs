//! LSM-tree core engine components.
//!
//! This module contains the central storage engine building blocks:
//!
//! - **`engine`** — the main [`Engine`](crate::core::engine::Engine) struct, compaction policy,
//!   version set, and transaction support.
//! - **`memtable`** — in-memory write buffer backed by a skiplist.
//! - **`table`** — in-memory representation of an SSTable for the version set.
//! - **`log_record`** — the key-value record format used throughout the engine.
//! - **`key`** — key encoding utilities.
//! - **`iterators`** — merge iterator that combines multiple sorted iterators.

pub mod engine;
pub mod iterators;
pub mod key;
pub mod log_record;
pub mod memtable;
pub mod table;
