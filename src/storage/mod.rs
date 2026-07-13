//! SSTable V2 persistent storage layer.
//!
//! Implements the SSTable V2 on-disk format with:
//!
//! - **`builder`** / **`reader`** — SSTable construction and read-back.
//! - **`block`** — data block encoding/decoding with optional prefix compression.
//! - **`wal`** — Write-Ahead Log for durability and crash recovery.
//! - **`cache`** — LRU block cache for hot data.
//! - **`encryption`** — AES-GCM encryption at rest.
//! - **`prefix_compression`** — block-level key prefix compression.
//! - **`config`** — storage configuration parameters.
//! - **`iterator`** — SSTable iterator for full-table scans.

pub mod block;
pub mod builder;
pub mod cache;
pub mod config;
pub mod encryption;
pub mod iterator;
pub mod prefix_compression;
pub mod reader;
pub mod wal;
