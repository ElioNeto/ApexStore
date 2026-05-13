pub mod auth;
pub mod config;

pub use self::config::ServerConfig;
use crate::LsmEngine;
use actix_web::{get, middleware::Logger, web, App, HttpResponse, HttpServer, Responder};
use serde_json::json;

/// Handler for `GET /keys`.
/// Returns a JSON object containing an array of all keys (bounded by `MAX_SCAN_LIMIT`).
#[get("/keys")]
async fn get_keys(engine: web::Data<LsmEngine>) -> impl Responder {
    // `LsmEngine::keys` applies the safety bound (MAX_SCAN_LIMIT).
    match engine.keys() {
        Ok(keys) => HttpResponse::Ok()
            .content_type("application/json")
            .json(json!({ "keys": keys })),
        Err(e) => {
            tracing::error!(target: "apexstore::api", "Failed to fetch keys: {:?}", e);
            HttpResponse::InternalServerError()
                .content_type("application/json")
                .json(json!({ "error": "internal server error" }))
        }
    }
}

/// Register API routes.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(get_keys);
}

/// Start the REST API server.
pub async fn start_server(engine: LsmEngine, config: ServerConfig) -> std::io::Result<()> {
    let host = config.host.clone();
    let port = config.port;

    tracing::info!(target: "apexstore::api", "Starting server at {}:{}", host, port);
    println!("🚀 Starting server at http://{}:{}", host, port);

    let engine_data = web::Data::new(engine);

    HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(engine_data.clone())
            .configure(configure)
    })
    .bind((host, port))?
    .run()
    .await
}
