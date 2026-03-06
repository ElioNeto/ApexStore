//! # ApexStore
//!
//! A high-performance, embedded key-value store written in Rust, implementing the
//! **Log-Structured Merge-Tree (LSM-Tree)** architecture.
//!
//! ## Overview
//!
//! ApexStore is designed for write-intensive workloads, combining the durability
//! of write-ahead logging with the efficiency of LSM-Tree architecture.
//!
//! ### Key Features
//!
//! - **High Write Throughput**: Optimized for write-heavy applications with in-memory
//!   buffering and sequential disk writes (500K+ ops/sec)
//! - **Data Durability**: Write-ahead log (WAL) ensures zero data loss on crashes
//! - **Space Efficiency**: Block-based compression with LZ4 reduces storage by 2-4x
//! - **Fast Reads**: Bloom filters and block caching for efficient lookups
//! - **Production Ready**: Comprehensive error handling and monitoring
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────┐
//! │   Client    │
//! └──────┬──────┘
//!        │
//!        ▼
//! ┌─────────────┐      ┌──────────┐
//! │  LSM Engine │─────▶│   WAL    │ (Durability)
//! └──────┬──────┘      └──────────┘
//!        │
//!        ├─────────────┬──────────────┐
//!        ▼             ▼              ▼
//!   ┌─────────┐  ┌──────────┐  ┌──────────┐
//!   │MemTable │  │ SSTable  │  │ SSTable  │
//!   │(Memory) │  │  (Disk)  │  │  (Disk)  │
//!   └─────────┘  └──────────┘  └──────────┘
//! ```
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use apexstore::{LsmEngine, LsmConfig};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create engine with default configuration
//! let config = LsmConfig::default();
//! let mut engine = LsmEngine::open(config)?;
//!
//! // Write data
//! engine.put("user:1", b"Alice")?;
//! engine.put("user:2", b"Bob")?;
//!
//! // Read data
//! if let Some(record) = engine.get("user:1")? {
//!     println!("Value: {:?}", record.value);
//! }
//!
//! // Delete data
//! engine.delete("user:1")?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Modules
//!
//! - [`core`] - Core LSM engine and data structures
//! - [`storage`] - Storage layer (WAL, SSTable, Block)
//! - [`infra`] - Infrastructure (config, error handling, codec)
//! - [`features`] - Feature flags system
//! - [`api`] - REST API server (optional, requires "api" feature)
//!
//! ## Feature Flags
//!
//! - `api` - Enable REST API server with Actix-Web
//!
//! ## Performance
//!
//! - **Write**: 500K ops/sec (in-memory), 100K ops/sec (with WAL)
//! - **Read**: 1M ops/sec (MemTable), 50K ops/sec (SSTable)
//! - **Compression**: 2-4x with LZ4
//! - **Latency**: p50 < 2µs, p99 < 15µs
//!
//! ## See Also
//!
//! - [GitHub Repository](https://github.com/ElioNeto/ApexStore)
//! - [Configuration Guide](https://github.com/ElioNeto/ApexStore/blob/main/docs/CONFIGURATION.md)
//! - [API Documentation](https://github.com/ElioNeto/ApexStore/blob/main/docs/API.md)

pub mod core;
pub mod features;
pub mod infra;
pub mod storage;

#[cfg(feature = "api")]
pub mod api;

pub use crate::core::engine::LsmEngine;
pub use crate::core::log_record::LogRecord;
pub use crate::features::{FeatureClient, FeatureFlag, Features};
pub use crate::infra::config::{CoreConfig, LsmConfig, LsmConfigBuilder, StorageConfig};
pub use crate::infra::error::{LsmError, Result};
