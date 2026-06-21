//! HTTP API integration tests for the TTL (Time-To-Live) endpoints.
//!
//! Covers:
//! - PUT a key with `ttl_secs`
//! - GET /keys/{key}/ttl returns remaining TTL
//! - PATCH /keys/{key}/ttl updates the TTL
//! - Key without TTL returns `ttl_secs: null`
//! - 404 for non-existent key TTL
//!
//! Note: TTL metadata (`__ttl:{key}`) is written to the SSTable during flush.
//! Tests that need to read TTL metadata after PUT must flush first.

use apexstore::api::test_helpers;
use serde_json::json;

/// Helper: flush the engine via the admin endpoint.
async fn flush_engine(
    app: &mut impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    >,
) {
    let (status, body) = test_helpers::post_json(app, "/admin/flush", &json!({})).await;
    assert_eq!(status, 200, "Flush should succeed, got body: {:?}", body);
}

#[actix_web::test]
async fn test_put_key_with_ttl() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // PUT a key with ttl_secs=3600
    let (status, _body) = test_helpers::put_json(
        &mut app,
        "/keys/ttl-key-1",
        &json!({"value": "ttl-value-1", "ttl_secs": 3600}),
    )
    .await;
    assert_eq!(status, 200, "PUT with TTL should succeed");

    // Flush so TTL metadata is written to the SSTable
    flush_engine(&mut app).await;

    // GET /keys/{key}/ttl returns ttl_secs > 0
    let body: serde_json::Value = test_helpers::get_json(&mut app, "/keys/ttl-key-1/ttl").await;
    assert_eq!(
        body["key"], "ttl-key-1",
        "Response should echo back the key"
    );
    let ttl = body["ttl_secs"].as_u64();
    assert!(ttl.is_some(), "ttl_secs should be present (got {:?})", body);
    assert!(
        ttl.unwrap() > 0,
        "ttl_secs should be positive (got {})",
        ttl.unwrap()
    );
    assert_eq!(body["expired"], false, "Key should not be expired");
}

#[actix_web::test]
async fn test_get_ttl_for_key_without_ttl() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // PUT a key without ttl
    let (status, _) = test_helpers::put_json(
        &mut app,
        "/keys/no-ttl-key",
        &json!({"value": "no-ttl-value"}),
    )
    .await;
    assert_eq!(status, 200);

    // GET /keys/{key}/ttl should return ttl_secs as null
    let body: serde_json::Value = test_helpers::get_json(&mut app, "/keys/no-ttl-key/ttl").await;
    assert_eq!(body["key"], "no-ttl-key");
    assert!(
        body["ttl_secs"].is_null(),
        "ttl_secs should be null for keys without TTL (got {:?})",
        body["ttl_secs"]
    );
    assert_eq!(body["expired"], false);
}

#[actix_web::test]
async fn test_get_ttl_nonexistent_key_returns_404() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let app = test_helpers::test_app(engine).await;

    // GET /keys/{key}/ttl for a nonexistent key should return 404
    let req = actix_web::test::TestRequest::get()
        .uri("/keys/nonexistent/ttl")
        .to_request();
    let resp = actix_web::test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        404,
        "GET /keys/nonexistent/ttl should return 404"
    );

    let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
    assert_eq!(
        body["error"], "key not found",
        "Response should contain 'key not found' error"
    );
}

#[actix_web::test]
async fn test_update_ttl() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // PUT a key with an initial TTL of 3600 seconds
    let (status, _) = test_helpers::put_json(
        &mut app,
        "/keys/ttl-update-key",
        &json!({"value": "update-me", "ttl_secs": 3600}),
    )
    .await;
    assert_eq!(status, 200);

    // Flush so TTL metadata is written
    flush_engine(&mut app).await;

    // PATCH /keys/{key}/ttl with a new TTL
    let (status, body) = test_helpers::patch_json(
        &mut app,
        "/keys/ttl-update-key/ttl",
        &json!({"ttl_secs": 7200}),
    )
    .await;
    assert_eq!(status, 200, "PATCH TTL should succeed");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["key"], "ttl-update-key");
    assert_eq!(body["ttl_secs"], 7200, "TTL should be updated to 7200");

    // Flush again so the updated TTL metadata is persisted
    flush_engine(&mut app).await;

    // Verify the new TTL via GET
    let body: serde_json::Value =
        test_helpers::get_json(&mut app, "/keys/ttl-update-key/ttl").await;
    let ttl = body["ttl_secs"].as_u64();
    assert!(ttl.is_some(), "ttl_secs should be present after update");
    assert!(
        ttl.unwrap() > 3500,
        "Updated TTL should be close to 7200 (got {})",
        ttl.unwrap()
    );
}

#[actix_web::test]
async fn test_update_ttl_nonexistent_key_returns_404() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // PATCH /keys/{key}/ttl for a nonexistent key should return 404
    let (status, body) = test_helpers::patch_json(
        &mut app,
        "/keys/no-such-key/ttl",
        &json!({"ttl_secs": 3600}),
    )
    .await;
    assert_eq!(
        status, 404,
        "PATCH TTL on nonexistent key should return 404"
    );
    assert_eq!(
        body["error"], "key not found",
        "Response should contain 'key not found' error"
    );
}
