pub mod access_control;
pub mod admin;
pub mod audit_middleware;
pub mod auth;
pub mod config;
pub mod connection_guard;
pub mod events;
pub mod graphql;
pub mod health;
pub mod notes;
pub mod rate_limiter;
pub mod sync;
pub mod timeout_middleware;

use self::access_control::AccessControl;
pub use self::auth::{require_permission, Permission, TokenManager};
pub use self::config::ServerConfig;
use self::connection_guard::IpConnectionGuard;
pub use self::graphql::AppSchema;
use self::rate_limiter::{RateLimiter, RateLimiterState};
use crate::infra::access_control::AccessController;
use crate::infra::cdc::CdcPublisher;
use crate::infra::idempotency::IdempotencyMiddleware;
use crate::core::engine::transaction::Transaction;
use crate::storage::cache::GlobalBlockCache;
use crate::LsmEngine;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{
    body::MessageBody, delete, get, patch, post, put, web, App, Error, HttpRequest, HttpResponse,
    HttpServer, Responder,
};
use actix_web_httpauth::middleware::HttpAuthentication;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_actix_web::{GraphQLRequest, GraphQLResponse};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

// ── Transaction management ──────────────────────────────────────────────────

/// Manages active transactions for the REST API.
pub struct TransactionManager {
    /// Map from numeric transaction ID to active transaction.
    pub transactions: Mutex<HashMap<u64, Transaction<Arc<GlobalBlockCache>>>>,
    /// Counter for generating transaction IDs.
    next_id: AtomicU64,
}

impl Default for TransactionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionManager {
    pub fn new() -> Self {
        Self {
            transactions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Begin a new transaction and return its numeric ID.
    pub fn begin(&self, engine: &LsmEngine) -> u64 {
        let txn = engine.begin_transaction();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut map = self.transactions.lock().unwrap();
        map.insert(id, txn);
        id
    }

    /// Remove and return a transaction by ID (for operations that need to mutate it).
    /// This is used for put/delete operations that consume the transaction.
    pub fn take(&self, id: u64) -> Option<Transaction<Arc<GlobalBlockCache>>> {
        self.transactions.lock().unwrap().remove(&id)
    }
}

/// Maximum number of records accepted in a single batch insert request.
pub const MAX_BATCH_SIZE: usize = 1000;

/// Query parameters for `GET /keys`
#[derive(Deserialize)]
pub struct KeysQuery {
    prefix: Option<String>,
    limit: Option<usize>,
    /// Query string (used by frontend for prefix search).
    #[allow(dead_code)]
    q: Option<String>,
    /// Cursor for paginated results (returned by previous response).
    cursor: Option<String>,
}

/// Query parameters for `GET /keys/search` with search mode.
#[derive(Deserialize)]
pub struct SearchQuery {
    /// Search query string (for prefix/contains/suffix/regex modes)
    q: Option<String>,
    /// Search mode: "prefix" (default), "contains", "suffix", "regex"
    mode: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    prefix: Option<String>,
}

/// Query parameters for `GET /keys/value-search`.
#[derive(Deserialize)]
pub struct ValueSearchQuery {
    q: Option<String>,
    limit: Option<usize>,
}

/// Query parameters for `GET /scan` with pagination and range bounds.
#[derive(Deserialize)]
pub struct ScanQuery {
    /// Lower bound (inclusive when include_lower is true)
    lower: Option<String>,
    /// Upper bound (exclusive when include_upper is false)
    upper: Option<String>,
    /// Maximum number of results (default 100, max MAX_SCAN_LIMIT)
    limit: Option<usize>,
    /// Cursor for paginated results
    cursor: Option<String>,
    /// Whether the lower bound is inclusive (default true)
    #[allow(dead_code)]
    #[serde(default = "default_true")]
    include_lower: bool,
    /// Whether the upper bound is inclusive (default false)
    #[allow(dead_code)]
    #[serde(default)]
    include_upper: bool,
}

#[allow(dead_code)]
fn default_true() -> bool { true }

/// Request body for `PUT /keys/{key}`
#[derive(Deserialize)]
pub struct SetBody {
    value: String,
    /// Optional TTL in seconds. When set, the key will auto-expire.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// Handler for `GET /keys/{key}` — get a single key.
#[get("/keys/{key}")]
async fn get_key(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let key = path.into_inner();
    match engine.get_cf("default", key.as_bytes()) {
        Ok(Some(value)) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "key": key, "value": String::from_utf8_lossy(&value) })),
        Ok(None) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "key not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to get key: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `PUT /keys/{key}` — upsert a key.
#[put("/keys/{key}")]
async fn put_key(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
    body: web::Json<SetBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let key = path.into_inner();

    // Validate key
    if key.is_empty() {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .json(json!({ "error": "key must not be empty" }));
    }
    if key.len() > 4096 {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .json(json!({ "error": "key too long" }));
    }

    let ttl = body.ttl_secs.map(std::time::Duration::from_secs);

    let result = if let Some(ttl) = ttl {
        engine.set_cf_with_ttl("default", key.as_bytes().to_vec(), body.value.as_bytes().to_vec(), ttl)
    } else {
        engine.put_cf(
            "default",
            key.as_bytes().to_vec(),
            body.value.as_bytes().to_vec(),
        )
    };

    match result {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "PUT key={} size={} ttl_secs={:?}",
                key,
                body.value.len(),
                body.ttl_secs
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "status": "ok" }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to set key: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /keys/{key}/ttl` — get remaining TTL for a key.
#[get("/keys/{key}/ttl")]
async fn get_key_ttl(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let key = path.into_inner();

    // Check if key exists
    match engine.get_cf("default", key.as_bytes()) {
        Ok(Some(_value)) => {
            // Look up TTL metadata: __ttl:{key}
            let ttl_key = format!("__ttl:{}", key);
            match engine.get_cf("default", ttl_key.as_bytes()) {
                Ok(Some(ttl_raw)) if ttl_raw.len() == 16 => {
                    let expires_at = u128::from_le_bytes(
                        ttl_raw.as_slice().try_into().unwrap_or(u128::MAX.to_le_bytes()),
                    );
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    if now >= expires_at {
                        return HttpResponse::Ok()
                            .content_type("application/json")
                            .json(json!({ "key": key, "ttl_secs": 0, "expired": true }));
                    }
                    let remaining_ns = expires_at - now;
                    let remaining_secs = remaining_ns / 1_000_000_000;
                    HttpResponse::Ok()
                        .content_type("application/json")
                        .json(json!({ "key": key, "ttl_secs": remaining_secs as u64, "expired": false }))
                }
                _ => {
                    // Key exists but no TTL metadata → no expiry
                    HttpResponse::Ok()
                        .content_type("application/json")
                        .json(json!({ "key": key, "ttl_secs": null, "expired": false }))
                }
            }
        }
        Ok(None) => HttpResponse::NotFound()
            .content_type("application/json")
            .json(json!({ "error": "key not found" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to get key: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `PATCH /keys/{key}/ttl` — update TTL on an existing key.
#[patch("/keys/{key}/ttl")]
async fn update_key_ttl(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
    body: web::Json<TtlUpdateBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let key = path.into_inner();

    // Read existing value
    let value = match engine.get_cf("default", key.as_bytes()) {
        Ok(Some(v)) => v,
        Ok(None) => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "key not found" }));
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to get key: {:?}", e);
            return HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }));
        }
    };

    // Re-write with new TTL
    let ttl = std::time::Duration::from_secs(body.ttl_secs);
    match engine.set_cf_with_ttl("default", key.as_bytes().to_vec(), value, ttl) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "UPDATE TTL key={} ttl_secs={}",
                key,
                body.ttl_secs
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "status": "ok", "key": key, "ttl_secs": body.ttl_secs }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to update TTL: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `POST /keys/batch/delete` — delete a batch of specific keys.
#[post("/keys/batch/delete")]
async fn batch_delete_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    body: web::Json<BatchDeleteBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Delete) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    if body.keys.is_empty() {
        return HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "deleted_count": 0 }));
    }

    let count = body.keys.len();
    match engine.delete_batch_cf("default", &body.keys.iter().map(|k| k.as_bytes()).collect::<Vec<_>>()) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "BATCH DELETE {} keys",
                count
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "deleted_count": count }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to batch delete keys: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `DELETE /keys?prefix=...` — delete all keys matching a prefix.
#[delete("/keys")]
async fn delete_keys_by_prefix(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<KeysQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Delete) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let prefix = match query.prefix {
        Some(ref p) if !p.is_empty() => p.clone(),
        _ => {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "error": "prefix query parameter is required" }));
        }
    };

    // Calculate upper bound for prefix
    let upper_bound = crate::core::engine::Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end(&prefix);

    // Use range tombstone for efficient prefix deletion
    match engine.delete_range_cf("default", prefix.as_bytes(), upper_bound.as_deref().unwrap_or(b"")) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "DELETE keys by prefix={}",
                prefix
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "status": "ok", "prefix": prefix }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to delete keys by prefix: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `DELETE /keys/{key}` — delete a key.
#[delete("/keys/{key}")]
async fn delete_key(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Delete) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let key = path.into_inner();
    match engine.delete_cf("default", key.as_bytes()) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "DELETE key={}",
                key
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "status": "ok" }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to delete key: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /keys` — list keys with optional prefix, cursor, lower/upper bounds, and limit.
#[get("/keys")]
async fn get_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<KeysQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let limit = query
        .limit
        .unwrap_or(100)
        .min(crate::core::engine::MAX_SCAN_LIMIT);

    let cursor = query.cursor.as_deref();

    if let Some(ref prefix) = query.prefix {
        let (results, new_cursor) = match engine.search_prefix(prefix, cursor, limit) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(target: "apexstore::api", "Failed to search prefix: {:?}", e);
                return HttpResponse::InternalServerError()
                    .content_type("application/json")
                    .json(json!({ "error": "internal server error" }));
            }
        };
        let keys: Vec<String> = results
            .into_iter()
            .map(|(k, _)| String::from_utf8_lossy(&k).to_string())
            .collect();
        return HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "keys": keys, "cursor": new_cursor }));
    }

    match engine.keys() {
        Ok(keys) => {
            let limited: Vec<String> = keys
                .into_iter()
                .take(limit)
                .map(|k| String::from_utf8_lossy(&k).to_string())
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "keys": limited }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to fetch keys: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /keys/range` — composite key range query with cursor pagination.
#[get("/keys/range")]
async fn keys_range(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<ScanQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let limit = query
        .limit
        .unwrap_or(100)
        .min(crate::core::engine::MAX_SCAN_LIMIT);

    // Determine lower bound: cursor takes precedence, then explicit lower
    let lower: Option<Vec<u8>> = if let Some(ref cursor) = query.cursor {
        Some(cursor.as_bytes().to_vec())
    } else {
        query.lower.as_ref().map(|s| s.as_bytes().to_vec())
    };

    // Adjust bounds for inclusivity
    let adjusted_lower: Option<&[u8]> = lower.as_deref();
    let adjusted_upper: Option<&[u8]> = query.upper.as_ref().map(|s| s.as_bytes());

    // Request extra records to detect if there are more results
    let scan_limit = limit + 1;

    match engine.scan_cf("default", adjusted_lower, adjusted_upper, Some(scan_limit)) {
        Ok(records) => {
            // If cursor is set, skip the first result if it matches the cursor key
            let records: Vec<(Vec<u8>, Vec<u8>)> = records
                .into_iter()
                .skip_while(|(k, _)| {
                    query.cursor.is_some()
                        && query.cursor.as_deref().is_some_and(|c| k.as_slice() == c.as_bytes())
                })
                .collect();

            let has_more = records.len() > limit;

            let mut records = records;
            records.truncate(limit);

            let new_cursor = if has_more {
                records
                    .last()
                    .and_then(|(k, _)| String::from_utf8(k.clone()).ok())
            } else {
                None
            };

            let records_json: Vec<serde_json::Value> = records
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();

            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({
                    "keys": records_json,
                    "cursor": new_cursor,
                    "has_more": has_more
                }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to query range: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /metrics`.
/// Returns Prometheus-formatted engine metrics.
#[get("/metrics")]
async fn get_metrics(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let metrics = engine.metrics();
    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(metrics.format_prometheus())
}

/// Handler for `GET /stats` — engine statistics.
#[get("/stats")]
async fn get_stats(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    match engine.stats("default") {
        Ok(stats) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({
                "sst_files": stats.sst_files,
                "sst_kb": stats.sst_kb,
                "mem_records": stats.mem_records,
                "mem_kb": stats.mem_kb,
                "wal_kb": stats.wal_kb,
                "total_records": stats.total_records,
                "max_levels_reached": stats.max_levels_reached,
            })),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to get stats: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /admin/rate_limits` — view current rate limit state.
#[get("/admin/rate_limits")]
async fn admin_rate_limits(
    req: HttpRequest,
    rate_limiter: web::Data<RateLimiterState>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Admin) {
        return e;
    }
    let summary = rate_limiter.get_state();
    HttpResponse::Ok()
        .content_type("application/json")
        .json(summary)
}

/// Handler for `POST /admin/flush` — force memtable flush.
#[post("/admin/flush")]
async fn admin_flush(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Admin) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    match engine.flush_memtable() {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Flush failed: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "flush failed" }))
        }
    }
}

/// Handler for `POST /admin/compact` — force compaction.
#[post("/admin/compact")]
async fn admin_compact(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Admin) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    match engine.compact() {
        Ok(results) => {
            let summaries: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(cf, m)| {
                    json!({
                        "cf": cf,
                        "files_merged": m.files_merged,
                        "bytes_read": m.bytes_read,
                        "bytes_written": m.bytes_written,
                    })
                })
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "compactions": summaries }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Compact failed: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "compaction failed" }))
        }
    }
}

// ── GraphQL handlers ────────────────────────────────────────────────────────

/// GraphQL endpoint — handles all queries and mutations.
async fn graphql_handler(schema: web::Data<AppSchema>, req: GraphQLRequest) -> GraphQLResponse {
    let res = schema.execute(req.into_inner()).await;
    GraphQLResponse::from(res)
}

/// GraphQL playground (interactive IDE).
///
/// Only available when `ENVIRONMENT=development` is set. Returns 404 otherwise.
async fn graphql_playground() -> HttpResponse {
    if std::env::var("ENVIRONMENT").as_deref() != Ok("development") {
        return HttpResponse::NotFound()
            .content_type("application/json")
            .body(r#"{"error":"not found"}"#);
    }
    let html = playground_source(
        GraphQLPlaygroundConfig::new("/graphql").title("ApexStore GraphQL Playground"),
    );
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

// ── Frontend-compatibility endpoints ─────────────────────────────────────

/// Request body for `POST /keys` (frontend-compatible key-value pair).
#[derive(Deserialize)]
pub struct FrontendSetBody {
    key: String,
    value: String,
    /// Optional TTL in seconds. When set, the key will auto-expire.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Request body for `POST /keys/batch`.
#[derive(Deserialize)]
pub struct BatchBody {
    records: Vec<FrontendSetBody>,
}

/// Request body for `POST /keys/batch/delete`.
#[derive(Deserialize)]
pub struct BatchDeleteBody {
    keys: Vec<String>,
}

/// Request body for `PATCH /keys/{key}/ttl`.
#[derive(Deserialize)]
pub struct TtlUpdateBody {
    /// New TTL in seconds from now.
    pub ttl_secs: u64,
}

/// Handler for `GET /stats/all` — frontend-compatible full stats endpoint.
///
/// ⚠️ **Deprecated**: This endpoint duplicates `GET /stats` with a different
/// response wrapper. Prefer `GET /stats` instead. This endpoint may be removed
/// in a future release.
#[get("/stats/all")]
async fn get_stats_all(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    match engine.stats("default") {
        Ok(stats) => HttpResponse::Ok()
            .insert_header(("Deprecation", "true"))
            .insert_header(("Sunset", "Sat, 31 Dec 2026 23:59:59 GMT"))
            .content_type("application/json")
            .json(json!({ "success": true, "data": {
                "mem_records": stats.mem_records,
                "mem_kb": stats.mem_kb,
                "sst_kb": stats.sst_kb,
                "sst_files": stats.sst_files,
                "wal_kb": stats.wal_kb,
                "total_records": stats.total_records,
            }})),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to get stats/all: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "success": false, "message": "internal server error" }))
        }
    }
}

/// Handler for `POST /keys` — frontend-compatible key insert (like PUT but with body key).
#[post("/keys")]
async fn post_key(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    body: web::Json<FrontendSetBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "success": false, "message": msg }));
    }

    // Validate key
    if body.key.is_empty() {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .json(json!({ "success": false, "message": "key must not be empty" }));
    }
    if body.key.len() > 4096 {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .json(json!({ "success": false, "message": "key too long" }));
    }

    let ttl = body.ttl_secs.map(std::time::Duration::from_secs);

    let result = if let Some(ttl) = ttl {
        engine.set_cf_with_ttl("default", body.key.as_bytes().to_vec(), body.value.as_bytes().to_vec(), ttl)
    } else {
        engine.put_cf(
            "default",
            body.key.as_bytes().to_vec(),
            body.value.as_bytes().to_vec(),
        )
    };

    match result {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "PUT key={} size={} ttl_secs={:?}",
                body.key,
                body.value.len(),
                body.ttl_secs
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "success": true, "data": { "key": body.key } }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to set key: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "success": false, "message": "internal server error" }))
        }
    }
}

// ── Transaction types ───────────────────────────────────────────────────────

/// Request body for `POST /txn/{txn_id}/put`.
#[derive(Deserialize)]
pub struct TxnPutBody {
    pub key: String,
    pub value: String,
    pub cf: Option<String>,
}

/// Request body for `POST /txn/{txn_id}/delete`.
#[derive(Deserialize)]
pub struct TxnDeleteBody {
    pub key: String,
    pub cf: Option<String>,
}

/// Handler for `POST /txn` — begin a new transaction.
#[post("/txn")]
async fn begin_txn(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    txn_mgr: web::Data<TransactionManager>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }
    let id = txn_mgr.begin(&engine);
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "txn_id": id }))
}

/// Handler for `POST /txn/{txn_id}/put` — stage a write in a transaction.
#[post("/txn/{txn_id}/put")]
async fn txn_put(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    txn_mgr: web::Data<TransactionManager>,
    path: web::Path<u64>,
    body: web::Json<TxnPutBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }
    let txn_id = path.into_inner();

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let mut txn = match txn_mgr.take(txn_id) {
        Some(t) => t,
        None => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "transaction not found" }));
        }
    };

    let cf = body.cf.as_deref().unwrap_or("default");
    if let Err(e) = txn.put_cf(cf, body.key.as_bytes(), body.value.as_bytes()) {
        tracing::error!(target: "apexstore::api", "txn put failed: {:?}", e);
        // Re-insert the transaction even on error
        txn_mgr.transactions.lock().unwrap().insert(txn_id, txn);
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .json(json!({ "error": "internal server error" }));
    }

    // Re-insert transaction after modification
    txn_mgr.transactions.lock().unwrap().insert(txn_id, txn);
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "status": "ok" }))
}

/// Handler for `POST /txn/{txn_id}/delete` — stage a delete in a transaction.
#[post("/txn/{txn_id}/delete")]
async fn txn_delete(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    txn_mgr: web::Data<TransactionManager>,
    path: web::Path<u64>,
    body: web::Json<TxnDeleteBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Delete) {
        return e;
    }
    let txn_id = path.into_inner();

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "error": msg }));
    }

    let mut txn = match txn_mgr.take(txn_id) {
        Some(t) => t,
        None => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "transaction not found" }));
        }
    };

    let cf = body.cf.as_deref().unwrap_or("default");
    if let Err(e) = txn.delete_cf(cf, body.key.as_bytes()) {
        tracing::error!(target: "apexstore::api", "txn delete failed: {:?}", e);
        txn_mgr.transactions.lock().unwrap().insert(txn_id, txn);
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .json(json!({ "error": "internal server error" }));
    }

    txn_mgr.transactions.lock().unwrap().insert(txn_id, txn);
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "status": "ok" }))
}

/// Handler for `POST /txn/{txn_id}/commit` — atomically commit a transaction.
#[post("/txn/{txn_id}/commit")]
async fn txn_commit(
    req: HttpRequest,
    txn_mgr: web::Data<TransactionManager>,
    path: web::Path<u64>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }
    let txn_id = path.into_inner();

    let mut txn = match txn_mgr.take(txn_id) {
        Some(t) => t,
        None => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "transaction not found" }));
        }
    };

    if let Err(e) = txn.commit() {
        tracing::error!(target: "apexstore::api", "txn commit failed: {:?}", e);
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .json(json!({ "error": "commit failed" }));
    }
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "status": "ok", "txn_id": txn_id }))
}

/// Handler for `POST /txn/{txn_id}/rollback` — discard staged writes.
#[post("/txn/{txn_id}/rollback")]
async fn txn_rollback(
    req: HttpRequest,
    txn_mgr: web::Data<TransactionManager>,
    path: web::Path<u64>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }
    let txn_id = path.into_inner();

    let mut txn = match txn_mgr.take(txn_id) {
        Some(t) => t,
        None => {
            return HttpResponse::NotFound()
                .content_type("application/json")
                .json(json!({ "error": "transaction not found" }));
        }
    };

    txn.rollback();
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "status": "ok", "txn_id": txn_id }))
}

/// Handler for `GET /keys/search` — search keys with multiple modes (prefix, contains, suffix, regex).
#[get("/keys/search")]
async fn search_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<SearchQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }

    let limit = query
        .limit
        .unwrap_or(100)
        .min(crate::core::engine::MAX_SCAN_LIMIT);

    let query_str = query.q.as_deref().or(query.prefix.as_deref()).unwrap_or("");
    let mode = query.mode.as_deref().unwrap_or("prefix");

    match mode {
        "contains" | "substring" => {
            // Substring/key-contains search
            match engine.search_contains(query_str, limit) {
                Ok(results) => {
                    let records: Vec<serde_json::Value> = results
                        .into_iter()
                        .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                        .collect();
                    HttpResponse::Ok()
                        .content_type("application/json")
                        .json(json!({ "success": true, "data": { "records": records } }))
                }
                Err(e) => {
                    tracing::error!(target: "apexstore::api", "Failed to search contains: {:?}", e);
                    HttpResponse::InternalServerError()
                        .content_type("application/json")
                        .json(json!({ "error": "internal server error" }))
                }
            }
        }
        "regex" => {
            // Regex matching on keys
            let re = match regex_lite::Regex::new(query_str) {
                Ok(r) => r,
                Err(e) => {
                    return HttpResponse::BadRequest()
                        .content_type("application/json")
                        .json(json!({ "error": format!("invalid regex: {}", e) }));
                }
            };
            match engine.scan_cf("default", None, None, Some(10000)) {
                Ok(results) => {
                    let filtered: Vec<serde_json::Value> = results
                        .into_iter()
                        .filter(|(k, _)| {
                            re.is_match(&String::from_utf8_lossy(k))
                        })
                        .take(limit)
                        .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                        .collect();
                    HttpResponse::Ok()
                        .content_type("application/json")
                        .json(json!({ "success": true, "data": { "records": filtered } }))
                }
                Err(e) => {
                    tracing::error!(target: "apexstore::api", "Failed to scan for regex: {:?}", e);
                    HttpResponse::InternalServerError()
                        .content_type("application/json")
                        .json(json!({ "error": "internal server error" }))
                }
            }
        }
        _ => {
            // Default: prefix search (supports cursor pagination)
            let cursor = query.cursor.as_deref();
            let (results, new_cursor) = match engine.search_prefix(query_str, cursor, limit) {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!(target: "apexstore::api", "Failed to search keys: {:?}", e);
                    return HttpResponse::InternalServerError()
                        .content_type("application/json")
                        .json(json!({ "success": false, "message": "internal server error" }));
                }
            };
            let records: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "success": true, "data": { "records": records, "cursor": new_cursor } }))
        }
    }
}

// ── Secondary Index types ───────────────────────────────────────────────────

/// Request body for `POST /keys/{key}/index/{field}`.
#[derive(Deserialize)]
pub struct IndexQuery {
    /// Index field name(s), comma-separated for compound indexes.
    #[serde(rename = "index")]
    pub index: Option<String>,
    /// Value(s) to match, comma-separated for compound queries.
    pub eq: Option<String>,
    pub limit: Option<usize>,
}

/// Handler for `GET /keys/by-index` — query by secondary index.
#[get("/keys/by-index")]
async fn query_by_index(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<IndexQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }

    let field = match query.index {
        Some(ref f) => f.clone(),
        None => {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "error": "index parameter is required" }));
        }
    };

    let value = match query.eq {
        Some(ref v) => v.clone(),
        None => {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "error": "eq parameter is required" }));
        }
    };

    let limit = query.limit.unwrap_or(100).min(1000);

    match engine.query_index("default", &field, &value, limit) {
        Ok(results) => {
            let records: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "success": true, "data": { "records": records } }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Index query failed: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `POST /keys/{key}/index/{field}` — create a secondary index on a field.
#[post("/keys/{key}/index/{field}")]
async fn create_index(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }

    let (key, field) = path.into_inner();

    match engine.create_index("default", &field) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "CREATE INDEX key={} field={}",
                key,
                field
            );
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "status": "ok", "key": key, "field": field }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to create index: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /keys/indexes` — list all secondary indexes.
#[get("/keys/indexes")]
async fn list_indexes(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }

    match engine.list_indexes("default") {
        Ok(fields) => {
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "indexes": fields }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to list indexes: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `GET /keys/value-search` — search by value content.
#[get("/keys/value-search")]
async fn value_search_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<ValueSearchQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }

    let q = match query.q {
        Some(ref q) if !q.is_empty() => q.clone(),
        _ => {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "error": "q parameter is required" }));
        }
    };

    let limit = query.limit.unwrap_or(100).min(100); // Safety cap

    match engine.value_search(&q, limit) {
        Ok(results) => {
            let records: Vec<serde_json::Value> = results
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "success": true, "data": { "records": records } }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed value search: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Handler for `POST /keys/batch` — batch insert.
#[post("/keys/batch")]
async fn batch_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    body: web::Json<BatchBody>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Write) {
        return e;
    }

    // Reject writes when engine is in read-only mode
    if let Err(msg) = engine.degradation.check_write_allowed() {
        return HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({ "success": false, "message": msg }));
    }

    if body.records.len() > MAX_BATCH_SIZE {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .json(json!({ "success": false, "message": format!("batch size {} exceeds maximum of {}", body.records.len(), MAX_BATCH_SIZE) }));
    }

    // Validate all keys before processing
    for record in &body.records {
        if record.key.is_empty() {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "success": false, "message": "key must not be empty" }));
        }
        if record.key.len() > 4096 {
            return HttpResponse::BadRequest()
                .content_type("application/json")
                .json(json!({ "success": false, "message": format!("key too long: {} bytes (max 4096)", record.key.len()) }));
        }
    }

    let mut count = 0;
    for record in &body.records {
        let ttl = record.ttl_secs.map(std::time::Duration::from_secs);
        let result = if let Some(ttl) = ttl {
            engine.set_cf_with_ttl("default", record.key.as_bytes().to_vec(), record.value.as_bytes().to_vec(), ttl)
        } else {
            engine.put_cf(
                "default",
                record.key.as_bytes().to_vec(),
                record.value.as_bytes().to_vec(),
            )
        };
        if result.is_ok() {
            count += 1;
        }
    }
    tracing::info!(
        target: "apexstore::audit",
        "BATCH put {} records",
        count
    );
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "success": true, "data": { "count": count } }))
}

/// Handler for `GET /scan` — scan keys with pagination, range bounds, and cursor.
#[get("/scan")]
async fn scan_keys(
    req: HttpRequest,
    engine: web::Data<LsmEngine>,
    query: web::Query<ScanQuery>,
) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let limit = query
        .limit
        .unwrap_or(100)
        .min(crate::core::engine::MAX_SCAN_LIMIT);

    // Determine lower bound: cursor takes precedence, then explicit lower
    let lower: Option<Vec<u8>> = if let Some(ref cursor) = query.cursor {
        // If cursor is set, use it as the new lower bound
        Some(cursor.as_bytes().to_vec())
    } else {
        query.lower.as_ref().map(|s| s.as_bytes().to_vec())
    };

    // Adjust bounds for inclusivity
    let adjusted_lower: Option<&[u8]> = lower.as_deref();
    let adjusted_upper: Option<&[u8]> = query.upper.as_ref().map(|s| s.as_bytes());

    // Request extra records to detect if there are more results.
    // Always +1 to check for more pages beyond the current limit.
    let scan_limit = limit + 1;

    match engine.scan_cf("default", adjusted_lower, adjusted_upper, Some(scan_limit)) {
        Ok(records) => {
            // If cursor is set, skip the first result if it matches the cursor key
            let records: Vec<(Vec<u8>, Vec<u8>)> = records
                .into_iter()
                .skip_while(|(k, _)| {
                    query.cursor.is_some()
                        && query.cursor.as_deref().is_some_and(|c| k.as_slice() == c.as_bytes())
                })
                .collect();

            // Determine if there are more results
            let has_more = records.len() > limit;

            // Take only `limit` results
            let mut records = records;
            records.truncate(limit);
            let new_cursor = if has_more {
                records
                    .last()
                    .and_then(|(k, _)| String::from_utf8(k.clone()).ok())
            } else {
                None
            };

            let records_json: Vec<serde_json::Value> = records
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();

            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({
                    "success": true,
                    "data": {
                        "records": records_json,
                        "cursor": new_cursor,
                        "has_more": has_more
                    }
                }))
        }
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to scan keys: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "success": false, "message": "internal server error" }))
        }
    }
}

// ── Route configuration ───────────────────────────────────────────────────

/// Register API routes.
///
/// Specific routes MUST be registered before parameterised routes
/// (e.g. `/{key}`) so that actix-web's router matches them correctly.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(get_keys)
        // Specific key-list endpoints — register before /keys/{key}
        .service(search_keys) // GET /keys/search
        .service(value_search_keys) // GET /keys/value-search
        .service(query_by_index) // GET /keys/by-index
        .service(create_index) // POST /keys/{key}/index/{field}
        .service(list_indexes) // GET /keys/indexes
        .service(batch_keys) // POST /keys/batch
        .service(batch_delete_keys) // POST /keys/batch/delete
        .service(delete_keys_by_prefix) // DELETE /keys (with ?prefix=)
        .service(keys_range) // GET /keys/range
        // Transaction endpoints — register before /keys/{key}
        .service(begin_txn) // POST /txn
        .service(txn_put) // POST /txn/{txn_id}/put
        .service(txn_delete) // POST /txn/{txn_id}/delete
        .service(txn_commit) // POST /txn/{txn_id}/commit
        .service(txn_rollback) // POST /txn/{txn_id}/rollback
        // TTL-specific endpoints — register before parameterised /keys/{key}
        .service(get_key_ttl) // GET /keys/{key}/ttl
        .service(update_key_ttl) // PATCH /keys/{key}/ttl
        // Parameterised key endpoints — must come after specific /keys/* routes
        .service(get_key) // GET /keys/{key}
        .service(put_key) // PUT /keys/{key}
        .service(delete_key) // DELETE /keys/{key}
        .service(post_key) // POST /keys
        .service(scan_keys) // GET /scan
        // Metrics and stats
        .service(get_metrics) // GET /metrics
        .service(get_stats) // GET /stats
        .service(get_stats_all) // GET /stats/all
        // Admin endpoints
        .service(admin_flush)
        .service(admin_compact)
        .service(admin_rate_limits)
        .service(web::scope("/admin").configure(admin::configure))
        // Health endpoints (no auth required)
        .service(health::liveness)
        .service(health::readiness)
        .service(health::startup)
        .service(health::health_check)
        // Notes & Tags endpoints
        .configure(notes::configure)
        // GraphQL endpoints
        .route("/graphql", web::post().to(graphql_handler))
        .route("/graphql", web::get().to(graphql_handler))
        .route("/graphql/playground", web::get().to(graphql_playground))
        // WebSocket sync endpoint
        .service(sync::sync_handler)
        // Real-time event endpoints
        .service(events::ws_events) // GET /ws/events
        .service(events::sse_events); // GET /events
}

/// Middleware that ensures mutating requests have a JSON content type,
/// preventing simple CSRF attacks via form-encoded submissions.
///
/// CSRF is not a primary concern for this API since we use Bearer token
/// authentication (not session cookies), which is inherently immune to
/// CSRF. This guard provides defense-in-depth for mutating endpoints.
struct ContentTypeGuard;

impl<S, B> Transform<S, ServiceRequest> for ContentTypeGuard
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Transform = ContentTypeGuardMiddleware<S>;
    type InitError = ();
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ContentTypeGuardMiddleware { service }))
    }
}

struct ContentTypeGuardMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for ContentTypeGuardMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        // Only check mutating methods
        if req.method() == actix_web::http::Method::PUT
            || req.method() == actix_web::http::Method::POST
            || req.method() == actix_web::http::Method::DELETE
        {
            let content_type = req
                .headers()
                .get(actix_web::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            // Allow only JSON content types for mutating requests
            if !content_type.starts_with("application/json") {
                return Box::pin(ready(Err(actix_web::error::ErrorUnsupportedMediaType(
                    "Content-Type must be application/json for mutating requests",
                ))));
            }
        }
        Box::pin(self.service.call(req))
    }
}

/// Build CORS middleware from configuration.
/// When disabled, returns a restrictive CORS policy that blocks all cross-origin
/// requests (default-deny). When enabled, either allows specific origins or all
/// origins depending on the `origins` parameter.
fn build_cors(origins: &Option<Vec<String>>, enabled: bool) -> actix_cors::Cors {
    if !enabled {
        return actix_cors::Cors::default()
            .max_age(0)
            .allowed_origin_fn(|_, _| false);
    }
    let mut cors = match origins {
        Some(origin_list) => {
            let mut c = actix_cors::Cors::default()
                .supports_credentials()
                .max_age(3600);
            for origin in origin_list {
                c = c.allowed_origin(origin);
            }
            c
        }
        None => {
            // Default-deny when no origins are configured — blocks all cross-origin requests
            actix_cors::Cors::default()
                .max_age(0)
                .allowed_origin_fn(|_, _| false)
        }
    };
    cors = cors
        .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS"])
        .allowed_headers(vec![
            actix_web::http::header::AUTHORIZATION,
            actix_web::http::header::CONTENT_TYPE,
            actix_web::http::header::ACCEPT,
        ])
        .expose_headers(vec!["x-request-id"]);
    cors
}

/// Start the REST API server.
///
/// Registers SIGINT and SIGTERM handlers so that `engine.close()` is called
/// before the server shuts down, ensuring WALs are synced and compaction
/// finishes cleanly.
pub async fn start_server(engine: Arc<LsmEngine>, config: ServerConfig) -> std::io::Result<()> {
    let host = config.host.clone();
    let port = config.port;

    // Build TLS config if enabled
    let tls_config = if config.tls_enabled {
        let cert_path = config.tls_cert_path.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TLS enabled but TLS_CERT_PATH not set",
            )
        })?;
        let key_path = config.tls_key_path.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TLS enabled but TLS_KEY_PATH not set",
            )
        })?;

        use std::io::BufReader;

        let cert_file = std::fs::File::open(cert_path).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Cannot open cert file: {}", e),
            )
        })?;
        let key_file = std::fs::File::open(key_path).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Cannot open key file: {}", e),
            )
        })?;

        let mut cert_reader = BufReader::new(cert_file);
        let mut key_reader = BufReader::new(key_file);

        let raw_certs: Vec<Vec<u8>> = rustls_pemfile::certs(&mut cert_reader).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse cert: {}", e),
            )
        })?;
        let certs: Vec<rustls::Certificate> =
            raw_certs.into_iter().map(rustls::Certificate).collect();

        // Read private key (PKCS#8 format)
        let mut raw_keys = rustls_pemfile::pkcs8_private_keys(&mut key_reader).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse key: {}", e),
            )
        })?;
        if raw_keys.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No private key found in key file (PKCS#8)",
            ));
        }
        let key = rustls::PrivateKey(raw_keys.remove(0));

        let tls_server_config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("TLS config error: {}", e),
                )
            })?;

        Some(tls_server_config)
    } else {
        None
    };

    if config.tls_enabled {
        tracing::info!(target: "apexstore::api", "HTTPS server listening on {}:{}", host, config.tls_port);
        println!("Starting server at https://{}:{}", host, config.tls_port);
    } else {
        tracing::info!(target: "apexstore::api", "HTTP server listening on {}:{}", host, port);
        println!("Starting server at http://{}:{}", host, port);
    }

    // Validate configuration and log warnings
    for warning in config.validate() {
        tracing::warn!(target: "apexstore::api", "Configuration warning: {}", warning);
    }

    // Create EventBus for WebSocket/SSE real-time event streaming
    let event_bus_inner = events::EventBus::new();
    event_bus_inner.set_enabled(true);
    let event_bus = web::Data::new(event_bus_inner.clone());

    // If a webhook endpoint is configured, chain both publishers
    let cdc_publisher: Box<dyn CdcPublisher> = if let Some(ref endpoint) = config.cdc_endpoint {
        let webhook_config = crate::infra::cdc::CdcConfig::with_endpoint(endpoint.clone());
        let webhook_publisher = crate::infra::cdc::create_publisher(&webhook_config)
            .expect("webhook publisher should be created");
        tracing::info!(target: "apexstore::api", "CDC webhook enabled, endpoint: {}", endpoint);
        Box::new(crate::infra::cdc::MultiPublisher::new(vec![
            Box::new(event_bus_inner.clone()),
            webhook_publisher,
        ]))
    } else {
        Box::new(event_bus_inner.clone())
    };

    engine.set_cdc_publisher(cdc_publisher);

    let engine_data = web::Data::from(engine.clone());
    let mut rl_state = RateLimiterState::new(config.rate_limit_requests_per_minute);
    rl_state.set_endpoint_limit("/admin/compact", 5);
    rl_state.set_endpoint_limit("/admin/flush", 5);
    let rate_limiter_state = web::Data::new(rl_state);
    let token_manager = web::Data::new(TokenManager::new_with_engine(engine.clone()));
    let auth_enabled = web::Data::new(config.auth.enabled);
    let graphql_schema = web::Data::new(graphql::build_schema(engine.clone()));
    let note_engine = web::Data::new(crate::notes::NoteEngine::new(engine.clone()));
    let time_travel_engine = web::Data::new(Mutex::new(
        crate::infra::time_travel::TimeTravelEngine::new(100),
    ));
    let sync_manager = web::Data::new(sync::SyncManager::new());
    let txn_manager = web::Data::new(TransactionManager::new());
    let idempotency = web::Data::new(IdempotencyMiddleware::new(Duration::from_secs(3600)));
    let ip_connection_guard = web::Data::new(IpConnectionGuard::new(config.max_connections_per_ip));

    let cors_enabled = config.cors_enabled;
    let cors_origins = config.cors_origins.clone();

    // Shared access control state
    let access_controller = web::Data::new(AccessController::new());
    let access_control_enabled = web::Data::new(config.access_control_enabled);

    let mut server_builder = HttpServer::new(move || {
        // CSRF protection is handled by the Bearer token authentication middleware.
        // Since the API uses stateless token auth (not session cookies), it is
        // inherently immune to CSRF attacks. The ContentTypeGuard below provides
        // defense-in-depth by rejecting non-JSON content types on mutating requests.
        let app = App::new()
            .wrap(self::ContentTypeGuard)
            .wrap(self::timeout_middleware::RequestTimeout)
            .wrap(self::connection_guard::ConnectionLimiter)
            .wrap(RateLimiter)
            .wrap(AccessControl)
            .wrap(actix_web::middleware::Logger::new(
                r#"{"time":"%t","level":"%l","request_id":"%{x-request-id}xi","method":"%r","status":%s,"duration_ms":%D,"size":%b}"#,
            ))
            .wrap(self::audit_middleware::AuditMiddleware)
            .wrap(build_cors(&cors_origins, cors_enabled))
            .wrap(HttpAuthentication::bearer(self::auth::bearer_validator));

        app.app_data(engine_data.clone())
            .app_data(rate_limiter_state.clone())
            .app_data(token_manager.clone())
            .app_data(auth_enabled.clone())
            .app_data(graphql_schema.clone())
            .app_data(note_engine.clone())
            .app_data(time_travel_engine.clone())
            .app_data(sync_manager.clone())
            .app_data(txn_manager.clone())
            .app_data(event_bus.clone())
            .app_data(access_controller.clone())
            .app_data(access_control_enabled.clone())
            .app_data(idempotency.clone())
            .app_data(ip_connection_guard.clone())
            .configure(configure)
    })
    .max_connections(config.max_connections)
    .backlog(config.backlog);

    if let Some(tls_config) = tls_config {
        server_builder = server_builder.bind_rustls((host.clone(), config.tls_port), tls_config)?;
    } else {
        server_builder = server_builder.bind((host.clone(), port))?;
    }

    if let Some(workers) = config.workers {
        server_builder = server_builder.workers(workers);
    }

    let server = server_builder.run();

    let server_handle = server.handle();

    // Spawn a signal handler that waits for SIGINT (Ctrl+C) or SIGTERM,
    // calls engine.close() to sync WALs and join the compaction thread,
    // then gracefully stops the HTTP server.
    let signal_engine = engine.clone();
    tokio::spawn(async move {
        // Wait for SIGINT (cross-platform) or SIGTERM (Unix).
        #[cfg(unix)]
        {
            let mut term_signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("Failed to register SIGTERM handler");

            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received SIGINT (Ctrl+C), shutting down...");
                }
                _ = term_signal.recv() => {
                    tracing::info!("Received SIGTERM, shutting down...");
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Received shutdown signal, shutting down...");
        }

        // Sync WALs and wait for compaction to finish.
        signal_engine.close();
        tracing::info!("Engine closed, stopping HTTP server...");

        server_handle.stop(true).await;
    });

    server.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cors_disabled() {
        // Should not panic
        let _cors = build_cors(&None, false);
    }

    #[test]
    fn test_build_cors_permissive() {
        let _cors = build_cors(&None, true);
    }

    #[test]
    fn test_build_cors_with_specific_origins() {
        let origins = Some(vec![
            "https://myapp.com".to_string(),
            "https://admin.myapp.com".to_string(),
        ]);
        let _cors = build_cors(&origins, true);
    }

    #[test]
    fn test_config_cors_defaults() {
        let config = ServerConfig::default();
        assert!(config.cors_enabled, "CORS should be enabled by default");
        assert!(
            config.cors_origins.is_none(),
            "CORS origins should be None by default"
        );
    }
}
