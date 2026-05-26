pub mod access_control;
pub mod admin;
pub mod audit_middleware;
pub mod auth;
pub mod config;
pub mod connection_guard;
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
use crate::infra::idempotency::IdempotencyMiddleware;
use crate::LsmEngine;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::{
    body::MessageBody, delete, get, post, put, web, App, Error, HttpRequest, HttpResponse,
    HttpServer, Responder,
};
use actix_web_httpauth::middleware::HttpAuthentication;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_actix_web::{GraphQLRequest, GraphQLResponse};
use serde::Deserialize;
use serde_json::json;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

/// Maximum number of records accepted in a single batch insert request.
pub const MAX_BATCH_SIZE: usize = 1000;

/// Query parameters for `GET /keys`
#[derive(Deserialize)]
pub struct KeysQuery {
    prefix: Option<String>,
    limit: Option<usize>,
    q: Option<String>,
}

/// Request body for `PUT /keys/{key}`
#[derive(Deserialize)]
pub struct SetBody {
    value: String,
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

    match engine.put_cf(
        "default",
        key.as_bytes().to_vec(),
        body.value.as_bytes().to_vec(),
    ) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "PUT key={} size={}",
                key,
                body.value.len()
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

/// Handler for `GET /keys` — list keys with optional prefix and limit.
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

    let result = if let Some(ref prefix) = query.prefix {
        let (results, _cursor) = match engine.search_prefix(prefix, None, limit) {
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
        serde_json::to_value(&keys).unwrap_or_default()
    } else {
        match engine.keys() {
            Ok(keys) => {
                let limited: Vec<String> = keys
                    .into_iter()
                    .take(limit)
                    .map(|k| String::from_utf8_lossy(&k).to_string())
                    .collect();
                serde_json::to_value(&limited).unwrap_or_default()
            }
            Err(e) => {
                tracing::error!(target: "apexstore::api", "Failed to fetch keys: {:?}", e);
                return HttpResponse::InternalServerError()
                    .content_type("application/json")
                    .json(json!({ "error": "internal server error" }));
            }
        }
    };

    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({ "keys": result }))
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
}

/// Request body for `POST /keys/batch`.
#[derive(Deserialize)]
pub struct BatchBody {
    records: Vec<FrontendSetBody>,
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

    match engine.put_cf(
        "default",
        body.key.as_bytes().to_vec(),
        body.value.as_bytes().to_vec(),
    ) {
        Ok(_) => {
            tracing::info!(
                target: "apexstore::audit",
                "PUT key={} size={}",
                body.key,
                body.value.len()
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

/// Handler for `GET /keys/search` — search keys by prefix or query.
#[get("/keys/search")]
async fn search_keys(
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
    // Use `q` if provided (frontend compatibility), otherwise fall back to `prefix`
    let prefix = query.q.as_deref().or(query.prefix.as_deref()).unwrap_or("");
    let (results, _cursor) = match engine.search_prefix(prefix, None, limit) {
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
        .json(json!({ "success": true, "data": { "records": records } }))
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
        if engine
            .put_cf(
                "default",
                record.key.as_bytes().to_vec(),
                record.value.as_bytes().to_vec(),
            )
            .is_ok()
        {
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

/// Handler for `GET /scan` — full scan of all keys.
#[get("/scan")]
async fn scan_keys(req: HttpRequest, engine: web::Data<LsmEngine>) -> impl Responder {
    if let Err(e) = require_permission(&req, Permission::Read) {
        return e;
    }
    let max_limit = 1000; // reasonable limit to prevent OOM
    match engine.scan_cf("default", None, None, Some(max_limit)) {
        Ok(records) => {
            let records: Vec<serde_json::Value> = records
                .into_iter()
                .map(|(k, v)| json!({ "key": String::from_utf8_lossy(&k), "value": String::from_utf8_lossy(&v) }))
                .collect();
            HttpResponse::Ok()
                .content_type("application/json")
                .json(json!({ "success": true, "data": { "records": records } }))
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
        .service(batch_keys) // POST /keys/batch
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
        .service(sync::sync_handler);
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

    // Configure CDC if an endpoint was provided
    if let Some(ref endpoint) = config.cdc_endpoint {
        let cdc_config = crate::infra::cdc::CdcConfig::with_endpoint(endpoint.clone());
        engine.set_cdc(cdc_config);
        tracing::info!(target: "apexstore::api", "CDC enabled, endpoint: {}", endpoint);
    }

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
