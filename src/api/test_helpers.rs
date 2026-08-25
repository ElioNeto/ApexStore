//! Test helpers for HTTP integration tests.
//!
//! Provides:
//! - `test_engine()` — creates an isolated `LsmEngine` with a temp directory
//! - `test_app()` — creates an actix-web test application with all API routes
//! - Convenience functions: `get_json`, `put_json`, `post_json`, `delete_json`, `patch_json`

use crate::api::auth::TokenManager;
use crate::api::connection_guard::IpConnectionGuard;
use crate::api::rate_limiter::RateLimiterState;
use crate::api::sync::SyncManager;
use crate::api::{configure, TransactionManager};
use crate::infra::access_control::AccessController;
use crate::infra::events::EventBus;
use crate::infra::idempotency::IdempotencyMiddleware;
use crate::infra::time_travel::TimeTravelEngine;
use crate::notes::NoteEngine;
use crate::storage::cache::GlobalBlockCache;
use crate::LsmEngine;
use actix_http::Request;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::{http::StatusCode, test, web, App};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;
use tempfile::TempDir;

/// Create a test engine with a temp directory and encryption disabled.
///
/// Returns the engine and a `TempDir` that is cleaned up when dropped.
/// The caller must keep the `TempDir` alive for the duration of the test.
///
/// # Example
///
/// ```rust
/// let (engine, _dir) = apexstore::api::test_helpers::test_engine();
/// // use engine...
/// ```
pub fn test_engine() -> (LsmEngine, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir should succeed");
    let mut config = crate::infra::config::LsmConfig::default();
    config.core.dir_path = dir.path().to_path_buf();
    // Disable encryption to avoid key file requirements in tests.
    // Note: EncryptionConfig::default() has enabled:true, so we also
    // set the encryption key to an empty string to ensure proper bypass.
    config.storage.encryption_enabled = false;
    config.storage.encryption_key_path = Some("/dev/null".to_string());
    let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(1, 4096))
        .expect("test engine should be created");
    (engine, dir)
}

/// Create an actix-web test `App` service with all API routes registered.
///
/// Authentication is **disabled** so that no bearer token is required.
/// The engine is wrapped in an `Arc` internally; pass `engine.clone()` if
/// you need to inspect engine state directly from the test.
///
/// # Example
///
/// ```rust
/// # async fn example() {
/// let (engine, _dir) = apexstore::api::test_helpers::test_engine();
/// let engine = std::sync::Arc::new(engine);
/// let mut app = apexstore::api::test_helpers::test_app(engine.clone()).await;
/// // send requests to `app` and verify via `engine`
/// # }
/// ```
pub async fn test_app(
    engine: Arc<LsmEngine>,
) -> impl Service<Request, Response = ServiceResponse, Error = actix_web::Error> {
    let engine_data: web::Data<LsmEngine> = web::Data::from(engine.clone());
    let rate_limiter_state = web::Data::new(RateLimiterState::new(10000));
    let token_manager = web::Data::new(TokenManager::default());
    let note_engine = web::Data::new(NoteEngine::new(engine.clone()));
    let txn_manager = web::Data::new(TransactionManager::new());
    let event_bus = web::Data::new(EventBus::new());
    event_bus.set_enabled(true);
    let sync_manager = web::Data::new(SyncManager::new());
    let time_travel_engine = web::Data::new(Mutex::new(TimeTravelEngine::new(100)));
    let auth_enabled = web::Data::new(crate::api::auth::AuthEnabled(false));
    let access_control_enabled =
        web::Data::new(crate::api::access_control::AccessControlEnabled(false));
    let access_controller = web::Data::new(AccessController::new());
    let idempotency = web::Data::new(IdempotencyMiddleware::new(std::time::Duration::from_secs(
        3600,
    )));
    let ip_connection_guard = web::Data::new(IpConnectionGuard::new(1000));

    test::init_service(
        App::new()
            .app_data(engine_data)
            .app_data(rate_limiter_state)
            .app_data(token_manager)
            .app_data(note_engine)
            .app_data(txn_manager)
            .app_data(event_bus)
            .app_data(sync_manager)
            .app_data(time_travel_engine)
            .app_data(auth_enabled)
            .app_data(access_control_enabled)
            .app_data(access_controller)
            .app_data(idempotency)
            .app_data(ip_connection_guard)
            .configure(configure),
    )
    .await
}

// ── Convenience HTTP helpers ─────────────────────────────────────────────

/// Send a GET request and return the JSON response body.
pub async fn get_json(
    app: &mut impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    url: &str,
) -> Value {
    let req = test::TestRequest::get().uri(url).to_request();
    let resp: ServiceResponse = test::call_service(app, req).await;
    test::read_body_json(resp).await
}

/// Send a PUT request with a JSON body and return `(status, body)`.
pub async fn put_json(
    app: &mut impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    url: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let req = test::TestRequest::put()
        .uri(url)
        .set_json(body)
        .to_request();
    let resp: ServiceResponse = test::call_service(app, req).await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    (status, body)
}

/// Send a POST request with a JSON body and return `(status, body)`.
pub async fn post_json(
    app: &mut impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    url: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let req = test::TestRequest::post()
        .uri(url)
        .set_json(body)
        .to_request();
    let resp: ServiceResponse = test::call_service(app, req).await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    (status, body)
}

/// Send a DELETE request and return `(status, body)`.
pub async fn delete_json(
    app: &mut impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    url: &str,
) -> (StatusCode, Value) {
    let req = test::TestRequest::delete().uri(url).to_request();
    let resp: ServiceResponse = test::call_service(app, req).await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    (status, body)
}

/// Send a PATCH request with a JSON body and return `(status, body)`.
pub async fn patch_json(
    app: &mut impl Service<Request, Response = ServiceResponse, Error = actix_web::Error>,
    url: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let req = test::TestRequest::patch()
        .uri(url)
        .set_json(body)
        .to_request();
    let resp: ServiceResponse = test::call_service(app, req).await;
    let status = resp.status();
    let body: Value = test::read_body_json(resp).await;
    (status, body)
}
