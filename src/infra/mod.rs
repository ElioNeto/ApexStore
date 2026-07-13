//! Infrastructure layer — cross-cutting services and value-added features.
//!
//! This module provides production-grade infrastructure on top of the core
//! LSM-Tree engine:
//!
//! ## Data services
//! - **`replication`** — leader-follower async shipping with exponential backoff.
//! - **`cdc`** — Change Data Capture with webhook publishers and event collectors.
//! - **`data_sync`** — two-way synchronisation with diff computation.
//! - **`blob_store`** — chunked blob storage for large values.
//! - **`bulk_io`** — streaming JSON/CSV import and export.
//!
//! ## Query interfaces
//! - **`sql`** — SQL query engine (SELECT/INSERT/DELETE via sqlparser).
//! - **`models::multi_model`** — document, time-series, and graph model dispatcher.
//! - **`vector_index`** — ANN vector search with cosine similarity.
//! - **`time_travel`** — snapshot-based historical queries.
//! - **`schema_validation`** — JSON Schema validation on write.
//!
//! ## Reliability
//! - **`resilience`** — consolidated module: backpressure, circuit breaker,
//!   and degradation management for fault tolerance.
//! - **`health`** — consolidated module: disk monitoring, data scrubbing,
//!   and watchdog health checks.
//! - **`retry`** — async exponential backoff with jitter.
//! - **`idempotency`** — TTL-based idempotency cache.
//! - **`panic_recovery`** — catch_unwind wrapper with history.
//! - **`memory_limiter`** — allocation tracking with peak monitoring.
//!
//! ## Observability
//! - **`telemetry`** — OpenTelemetry tracing and metrics via OTLP.
//! - **`metrics`** — atomic counters, latency accumulators, Prometheus format.
//! - **`log`** — structured operation logging with TUI formatting.
//! - **`events`** — tokio broadcast-based event bus.
//!
//! ## Multi-tenancy & governance
//! - **`access_control`** — policy-based access engine with glob matching.
//! - **`quotas`** — per-tenant key/storage/rate quotas with sliding window.
//! - **`query_budget`** — per-query cost tracking (key reads, byte scans).
//!
//! ## Automation
//! - **`models`** — consolidated module: data tiering and multi-model query.
//! - **`pubsub`** — topic-based pub/sub via tokio broadcast.
//! - **`webhook_triggers`** — prefix-based webhooks backed by CDC.
//! - **`backup_scheduler`** — background backup with retention pruning.
//! - **`crdt`** — conflict-free replicated data types (LWW, GCounter, ORSet).
//! - **`chaos`** — fault injection for resilience testing.
//! - **`cicd`** — test fixture management and data seeding.
//!
//! ## Extensibility
//! - **`wasm_plugin`** — WebAssembly plugin runtime (requires `wasm` feature).
//! - **`codec`** — serialisation codecs (postcard).
//! - **`config`** — typed configuration with builder and env-var support.
//! - **`error`** — unified error types via thiserror.

pub mod access_control;
pub mod backup_scheduler;
pub mod blob_store;
pub mod bulk_io;
pub mod cdc;
pub mod chaos;
pub mod cicd;
pub mod codec;
pub mod config;
pub mod crdt;
pub mod data_sync;
pub mod error;
pub mod events;
pub mod health;
pub mod idempotency;
pub mod log;
pub mod memory_limiter;
pub mod metrics;
pub mod models;
pub mod panic_recovery;
pub mod pubsub;
pub mod query_budget;
pub mod quotas;
pub mod replication;
pub mod resilience;
pub mod retry;
pub mod schema_validation;
pub mod sql;
pub mod telemetry;
pub mod time_travel;
pub mod vector_index;
pub mod webhook_triggers;

// ── Differentiator features ────────────────────────────────────────────────

/// WebAssembly plugin system (requires `wasm` feature).
#[cfg(feature = "wasm")]
pub mod wasm_plugin;

// ── Re-exports ──────────────────────────────────────────────────────────────

pub use self::events::{EventBus, EventQuery};
