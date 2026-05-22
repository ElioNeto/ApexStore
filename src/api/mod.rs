pub mod auth;
pub mod config;
pub mod rate_limiter;

pub use self::config::ServerConfig;
use self::rate_limiter::{RateLimiter, RateLimiterState};
use crate::LsmEngine;
use actix_web::{delete, get, post, put, web, App, HttpResponse, HttpServer, Responder};
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
async fn get_key(engine: web::Data<LsmEngine>, path: web::Path<String>) -> impl Responder {
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
    engine: web::Data<LsmEngine>,
    path: web::Path<String>,
    body: web::Json<SetBody>,
) -> impl Responder {
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
async fn delete_key(engine: web::Data<LsmEngine>, path: web::Path<String>) -> impl Responder {
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
async fn get_keys(engine: web::Data<LsmEngine>, query: web::Query<KeysQuery>) -> impl Responder {
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
async fn get_metrics(engine: web::Data<LsmEngine>) -> impl Responder {
    let metrics = engine.metrics();
    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(metrics.format_prometheus())
}

/// Handler for `GET /stats` — engine statistics.
#[get("/stats")]
async fn get_stats(engine: web::Data<LsmEngine>) -> impl Responder {
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

/// Handler for `POST /admin/flush` — force memtable flush.
#[post("/admin/flush")]
async fn admin_flush(engine: web::Data<LsmEngine>) -> impl Responder {
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
async fn admin_compact(engine: web::Data<LsmEngine>) -> impl Responder {
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
        .service(admin_compact);
}

/// Start the REST API server.
///
/// Registers SIGINT and SIGTERM handlers so that `engine.close()` is called
/// before the server shuts down, ensuring WALs are synced and compaction
/// finishes cleanly.
pub async fn start_server(engine: Arc<LsmEngine>, config: ServerConfig) -> std::io::Result<()> {
    let host = config.host.clone();
    let port = config.port;

    tracing::info!(target: "apexstore::api", "Starting server at {}:{}", host, port);
    println!("Starting server at http://{}:{}", host, port);

    let engine_data = web::Data::from(engine.clone());
    let rate_limiter_state =
        web::Data::new(RateLimiterState::new(config.rate_limit_requests_per_minute));

    let mut server_builder = HttpServer::new(move || {
        App::new()
            .wrap(RateLimiter)
            .wrap(actix_web::middleware::Logger::default())
            .app_data(engine_data.clone())
            .app_data(rate_limiter_state.clone())
            .configure(configure)
    })
    .max_connections(config.max_connections)
    .backlog(config.backlog)
    .bind((host, port))?;

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
