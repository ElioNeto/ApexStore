pub mod api;
pub mod cli;
pub mod core;
pub mod features;
pub mod infra;
pub mod storage;

// Re-exports for convenience and backward compatibility
pub use crate::core::engine::{LsmEngine, LsmStats};
pub use crate::infra::access_control::{AccessController, AccessPolicy, Effect, Operation};
pub use crate::infra::blob_store::{BlobEngine, BlobStore, BlobStoreConfig};
pub use crate::infra::cdc::{CdcConfig, CdcEvent, CdcEventType, CdcPublisher};
pub use crate::infra::cicd::{Fixture, FixtureEntry, TestFixture};
pub use crate::infra::config::LsmConfig;
pub use crate::infra::crdt::{CrdtEngine, CrdtEntry};
pub use crate::infra::data_sync::{DataSync, DiffEntry, LocalEngine, RemoteBackend, SyncDirection};
pub use crate::infra::error::{LsmError, Result};
pub use crate::infra::log::{LogLevel, UsageEntry, UsageLog};
pub use crate::infra::query_budget::{BudgetExhausted, QueryBudget};
pub use crate::infra::replication::{
    ReplicationClient, ReplicationConfig, ReplicationFrame, ReplicationRole, ReplicationStats,
};
pub use crate::infra::schema_validation::{SchemaValidator, ValidationError};

// ── Differentiator features re-exports ────────────────────────────────────
#[cfg(feature = "wasm")]
pub use crate::infra::wasm_plugin::WasmPlugin;
pub use crate::infra::vector_index::VectorIndex;
pub use crate::infra::time_travel::TimeTravelEngine;
pub use crate::infra::pubsub::PubSub;
pub use crate::infra::data_tiering::{DataTieringConfig, Tier};
pub use crate::infra::multi_model::{MultiModelEngine, Document, TimeSeriesPoint, GraphVertex};
pub use crate::infra::webhook_triggers::WebhookRegistry;
