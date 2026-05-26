pub mod access_control;
pub mod admin;
pub mod auth;
pub mod config;
pub mod graphql;
pub mod health;
pub mod rate_limiter;
pub mod timeout_middleware;

use self::access_control::AccessControl;
pub use self::auth::{require_permission, Permission, TokenManager};
pub use self::config::ServerConfig;
pub use self::graphql::AppSchema;
use self::rate_limiter::{RateLimiter, RateLimiterState};
use crate::infra::access_control::AccessController;
use crate::LsmEngine;
use actix_web::{
    delete, get, post, put, web, App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_httpauth::middleware::HttpAuthentication;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_actix_web::{GraphQLRequest, GraphQLResponse};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Query parameters for `GET /keys`
#[derive(Deserialize)]
pub struct KeysQuery {
    prefix: Option<String>,
    limit: Option<usize>,
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
    let key = path.into_inner();
    match engine.put_cf(
        "default",
        key.as_bytes().to_vec(),
        body.value.as_bytes().to_vec(),
    ) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
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
    let key = path.into_inner();
    match engine.delete_cf("default", key.as_bytes()) {
        Ok(_) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "status": "ok" })),
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
async fn graphql_playground() -> HttpResponse {
    let html = playground_source(
        GraphQLPlaygroundConfig::new("/graphql").title("ApexStore GraphQL Playground"),
    );
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

// ── Route configuration ───────────────────────────────────────────────────

/// Register API routes.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(get_keys)
        .service(get_key)
        .service(put_key)
        .service(delete_key)
        .service(get_metrics)
        .service(get_stats)
        .service(admin_flush)
        .service(admin_compact)
        .service(admin_rate_limits)
        .service(web::scope("/admin").configure(admin::configure))
        // Health endpoints (no auth required)
        .service(health::liveness)
        .service(health::readiness)
        .service(health::startup)
        // GraphQL endpoints
        .route("/graphql", web::post().to(graphql_handler))
        .route("/graphql", web::get().to(graphql_handler))
        .route("/graphql/playground", web::get().to(graphql_playground));
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
        None => actix_cors::Cors::permissive(),
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

    // Configure CDC if an endpoint was provided
    if let Some(ref endpoint) = config.cdc_endpoint {
        let cdc_config = crate::infra::cdc::CdcConfig::with_endpoint(endpoint.clone());
        engine.set_cdc(cdc_config);
        tracing::info!(target: "apexstore::api", "CDC enabled, endpoint: {}", endpoint);
    }

    let engine_data = web::Data::from(engine.clone());
    let rate_limiter_state =
        web::Data::new(RateLimiterState::new(config.rate_limit_requests_per_minute));
    let token_manager = web::Data::new(TokenManager::new_with_engine(engine.clone()));
    let auth_enabled = web::Data::new(config.auth.enabled);
    let graphql_schema = web::Data::new(graphql::build_schema(engine.clone()));

    let cors_enabled = config.cors_enabled;
    let cors_origins = config.cors_origins.clone();

    // Shared access control state
    let access_controller = web::Data::new(AccessController::new());
    let access_control_enabled = web::Data::new(config.access_control_enabled);

    let mut server_builder = HttpServer::new(move || {
        let app = App::new()
            .wrap(self::timeout_middleware::RequestTimeout)
            .wrap(RateLimiter)
            .wrap(AccessControl)
            .wrap(actix_web::middleware::Logger::default())
            .wrap(build_cors(&cors_origins, cors_enabled))
            .wrap(HttpAuthentication::bearer(self::auth::bearer_validator));

        app.app_data(engine_data.clone())
            .app_data(rate_limiter_state.clone())
            .app_data(token_manager.clone())
            .app_data(auth_enabled.clone())
            .app_data(graphql_schema.clone())
            .app_data(access_controller.clone())
            .app_data(access_control_enabled.clone())
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
