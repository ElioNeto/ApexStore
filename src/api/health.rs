//! Health check endpoints for Kubernetes liveness, readiness, and startup probes.
//!
//! # Endpoints
//!
//! | Path                     | Purpose      | Returns 200 when …                        |
//! |--------------------------|--------------|-------------------------------------------|
//! | `GET /health/liveness`   | Liveness     | Always (server is alive)                  |
//! | `GET /health/readiness`  | Readiness    | Engine stats are accessible               |
//! | `GET /health/startup`    | Startup      | Engine fully initialized with default CF  |

use crate::LsmEngine;
use actix_web::{get, web, HttpResponse, Responder};
use serde_json::json;
use std::sync::LazyLock;
use std::time::Instant;

/// Server start time — used to compute uptime in `/health/check`.
static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Handler for `GET /health/liveness` — always returns 200.
///
/// Indicates the server process is alive and responding to HTTP requests.
#[get("/health/liveness")]
pub async fn liveness() -> impl Responder {
    HttpResponse::Ok()
        .content_type("application/json")
        .json(json!({
            "status": "ok",
            "service": "apexstore",
            "endpoint": "liveness"
        }))
}

/// Handler for `GET /health/readiness` — checks if the engine is ready to
/// accept requests.
///
/// Verifies engine stats are accessible (implies WAL is available, memtable is
/// initialised, etc.). Returns 503 if the engine is closing or unreachable.
#[get("/health/readiness")]
pub async fn readiness(engine: web::Data<LsmEngine>) -> impl Responder {
    match engine.stats("default") {
        Ok(stats) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({
                "status": "ok",
                "service": "apexstore",
                "endpoint": "readiness",
                "details": {
                    "sst_files": stats.sst_files,
                    "wal_kb": stats.wal_kb,
                    "mem_records": stats.mem_records,
                }
            })),
        Err(e) => HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({
                "status": "error",
                "service": "apexstore",
                "endpoint": "readiness",
                "reason": format!("engine stats unavailable: {}", e)
            })),
    }
}

/// Handler for `GET /health/startup` — checks if the engine has fully
/// initialised.
///
/// Verifies that the default column family exists and engine stats can be
/// queried.
#[get("/health/startup")]
pub async fn startup(engine: web::Data<LsmEngine>) -> impl Responder {
    match engine.stats("default") {
        Ok(stats) => {
            // Confirm the default CF is present via column_families()
            let cf_ok = {
                let core = engine.lock_core();
                core.version_set()
                    .column_families()
                    .iter()
                    .any(|cf| cf == "default")
            };

            if cf_ok {
                HttpResponse::Ok()
                    .content_type("application/json")
                    .json(json!({
                        "status": "ok",
                        "service": "apexstore",
                        "endpoint": "startup",
                        "details": {
                            "sst_files": stats.sst_files,
                            "wal_kb": stats.wal_kb,
                            "mem_records": stats.mem_records,
                        }
                    }))
            } else {
                HttpResponse::ServiceUnavailable()
                    .content_type("application/json")
                    .json(json!({
                        "status": "error",
                        "service": "apexstore",
                        "endpoint": "startup",
                        "reason": "default column family not found"
                    }))
            }
        }
        Err(e) => HttpResponse::ServiceUnavailable()
            .content_type("application/json")
            .json(json!({
                "status": "error",
                "service": "apexstore",
                "endpoint": "startup",
                "reason": format!("engine stats unavailable: {}", e)
            })),
    }
}

/// Handler for `GET /health/check` — comprehensive engine status.
///
/// Returns engine stats along with server uptime.
#[get("/health/check")]
pub async fn health_check(engine: web::Data<LsmEngine>) -> impl Responder {
    match engine.stats("default") {
        Ok(stats) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({
                "status": "ok",
                "service": "apexstore",
                "endpoint": "check",
                "uptime_secs": START_TIME.elapsed().as_secs(),
                "details": {
                    "memtable_records": stats.mem_records,
                    "sstable_files": stats.sst_files,
                    "wal_size_kb": stats.wal_kb,
                    "total_records": stats.total_records,
                }
            })),
        Err(e) => HttpResponse::InternalServerError()
            .content_type("application/json")
            .json(json!({
                "status": "error",
                "service": "apexstore",
                "endpoint": "check",
                "reason": format!("engine stats unavailable: {}", e)
            })),
    }
}
