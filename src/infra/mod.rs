pub mod access_control;
pub mod backpressure;
pub mod backup_scheduler;
pub mod blob_store;
pub mod bulk_io;
pub mod cdc;
pub mod chaos;
pub mod cicd;
pub mod circuit_breaker;
pub mod codec;
pub mod config;
pub mod crdt;
pub mod data_sync;
pub mod data_tiering;
pub mod degradation;
pub mod disk_monitor;
pub mod error;
pub mod idempotency;
pub mod log;
pub mod memory_limiter;
pub mod metrics;
pub mod multi_model;
pub mod panic_recovery;
pub mod pubsub;
pub mod query_budget;
pub mod quotas;
pub mod replication;
pub mod retry;
pub mod schema_validation;
pub mod scrubber;
pub mod sql;
pub mod telemetry;
pub mod time_travel;
pub mod vector_index;
pub mod watchdog;
pub mod webhook_triggers;

// ── Differentiator features ────────────────────────────────────────────────

/// WebAssembly plugin system (requires `wasm` feature).
#[cfg(feature = "wasm")]
pub mod wasm_plugin;
