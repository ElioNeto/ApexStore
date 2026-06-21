//! HTTP API integration tests for scan, pagination, range, search, batch-delete,
//! and secondary index endpoints.
//!
//! Covers:
//! - GET /scan (basic, with limit, with cursor pagination)
//! - GET /keys/range (lower/upper bounds)
//! - GET /keys/value-search
//! - POST /keys/batch/delete
//! - GET /keys/by-index (secondary index query)

use apexstore::api::test_helpers;
use serde_json::json;

/// Helper to put a key via the HTTP API and assert success.
async fn put_key(
    app: &mut impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    >,
    key: &str,
    value: &str,
) {
    let (status, _) = test_helpers::put_json(
        app,
        &format!("/keys/{}", key),
        &json!({"value": value}),
    )
    .await;
    assert_eq!(status, 200, "PUT /keys/{} should succeed", key);
}

// ── GET /scan ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn test_scan_basic() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // Insert several keys
    for i in 1..=5 {
        put_key(&mut app, &format!("scan-basic-{}", i), &format!("val-{}", i)).await;
    }

    // GET /scan
    let body: serde_json::Value = test_helpers::get_json(&mut app, "/scan").await;
    assert!(
        body.get("error").is_none(),
        "Scan should not return an error"
    );

    let records = body["data"]["records"].as_array().unwrap();
    assert!(!records.is_empty(), "Should return at least one record");
    assert!(records.len() <= 100, "Default limit should be 100");
}

#[actix_web::test]
async fn test_scan_with_limit() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    for i in 1..=10 {
        put_key(&mut app, &format!("scan-limit-{:02}", i), &format!("val-{}", i)).await;
    }

    // GET /scan?limit=3
    let body: serde_json::Value = test_helpers::get_json(&mut app, "/scan?limit=3").await;
    let records = body["data"]["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        3,
        "Should return exactly 3 records with limit=3"
    );
    let has_more = body["data"]["has_more"].as_bool().unwrap_or(false);
    assert!(has_more, "Should indicate more results available");
}

#[actix_web::test]
async fn test_scan_with_cursor_pagination() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    for i in 1..=10 {
        put_key(
            &mut app,
            &format!("scan-cursor-{:02}", i),
            &format!("val-{}", i),
        )
        .await;
    }

    // First page (limit=3)
    let body: serde_json::Value = test_helpers::get_json(&mut app, "/scan?limit=3").await;
    let records = body["data"]["records"].as_array().unwrap();
    assert_eq!(records.len(), 3, "First page should have 3 records");
    let cursor = body["data"]["cursor"]
        .as_str()
        .map(|s| s.to_string())
        .expect("Cursor should be present for pagination");

    // Second page using cursor
    let body2: serde_json::Value =
        test_helpers::get_json(&mut app, &format!("/scan?limit=3&cursor={}", cursor)).await;
    let records2 = body2["data"]["records"].as_array().unwrap();
    assert_eq!(records2.len(), 3, "Second page should have 3 records");

    // Verify keys are different between pages
    let first_keys: Vec<&str> = records
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    let second_keys: Vec<&str> = records2
        .iter()
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    for k in &second_keys {
        assert!(
            !first_keys.contains(k),
            "Second page should not contain keys from first page"
        );
    }
}

// ── GET /keys/range ────────────────────────────────────────────────────────

#[actix_web::test]
async fn test_keys_range_basic() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    for i in 1..=5 {
        put_key(
            &mut app,
            &format!("range-key-{}", i),
            &format!("rval-{}", i),
        )
        .await;
    }

    // GET /keys/range with lower and upper bounds
    let body: serde_json::Value = test_helpers::get_json(
        &mut app,
        "/keys/range?lower=range-key-2&upper=range-key-4",
    )
    .await;
    let keys = body["keys"].as_array().unwrap();
    assert!(!keys.is_empty(), "Range should return results");

    // Keys should be within bounds
    for entry in keys {
        let key = entry["key"].as_str().unwrap();
        assert!(
            key >= "range-key-2",
            "Key {} should be >= lower bound",
            key
        );
    }
}

#[actix_web::test]
async fn test_keys_range_with_cursor() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    for i in 1..=10 {
        put_key(
            &mut app,
            &format!("range-cur-{:02}", i),
            &format!("val-{}", i),
        )
        .await;
    }

    // GET /keys/range with limit=3, should return a cursor
    let body: serde_json::Value = test_helpers::get_json(
        &mut app,
        "/keys/range?lower=range-cur-01&upper=range-cur-10&limit=3",
    )
    .await;
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 3, "Range with limit=3 should return 3 entries");
    let cursor = body["cursor"].as_str();
    assert!(cursor.is_some(), "Cursor should be present for pagination");
    assert_eq!(
        body["has_more"], true,
        "has_more should be true when there are more results"
    );
}

// ── GET /keys/value-search ─────────────────────────────────────────────────

#[actix_web::test]
async fn test_value_search() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    put_key(&mut app, "vs-key-1", "hello world").await;
    put_key(&mut app, "vs-key-2", "hello there").await;
    put_key(&mut app, "vs-key-3", "goodbye").await;

    // GET /keys/value-search?q=hello
    let body: serde_json::Value =
        test_helpers::get_json(&mut app, "/keys/value-search?q=hello").await;
    assert!(
        body.get("error").is_none(),
        "Value search should not error"
    );

    let records = body["data"]["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        2,
        "Should find 2 keys with 'hello' in the value"
    );
}

#[actix_web::test]
async fn test_value_search_no_match() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    put_key(&mut app, "vs-nomatch", "unique value").await;

    // Search for a string that doesn't exist
    let body: serde_json::Value =
        test_helpers::get_json(&mut app, "/keys/value-search?q=zzzznonexistent").await;
    let records = body["data"]["records"].as_array().unwrap();
    assert!(
        records.is_empty(),
        "Should return empty results for non-matching search"
    );
}

// ── POST /keys/batch/delete ────────────────────────────────────────────────

#[actix_web::test]
async fn test_batch_delete() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    put_key(&mut app, "bd-key-1", "val1").await;
    put_key(&mut app, "bd-key-2", "val2").await;
    put_key(&mut app, "bd-key-3", "val3").await;

    // POST /keys/batch/delete
    let (status, body) = test_helpers::post_json(
        &mut app,
        "/keys/batch/delete",
        &json!({"keys": ["bd-key-1", "bd-key-3"]}),
    )
    .await;
    assert_eq!(status, 200, "Batch delete should succeed");
    assert_eq!(
        body["deleted_count"], 2,
        "Should report 2 deleted keys"
    );

    // Verify deletions via engine
    let val1 = engine.get("bd-key-1").unwrap();
    assert!(val1.is_none(), "bd-key-1 should be deleted");
    let val2 = engine.get("bd-key-2").unwrap();
    assert!(val2.is_some(), "bd-key-2 should still exist");
    let val3 = engine.get("bd-key-3").unwrap();
    assert!(val3.is_none(), "bd-key-3 should be deleted");
}

#[actix_web::test]
async fn test_batch_delete_empty_keys() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // POST /keys/batch/delete with empty keys list
    let (status, body) = test_helpers::post_json(
        &mut app,
        "/keys/batch/delete",
        &json!({"keys": []}),
    )
    .await;
    assert_eq!(status, 200, "Empty batch delete should succeed");
    assert_eq!(
        body["deleted_count"], 0,
        "Should report 0 deleted keys"
    );
}

// ── Secondary Index: POST /keys/{key}/index/{field} + GET /keys/by-index ──

#[actix_web::test]
async fn test_create_and_query_index() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // Create an index on the "status" field
    let (status, _body) = test_helpers::post_json(
        &mut app,
        "/keys/test-key/index/status",
        &json!({}),
    )
    .await;
    assert_eq!(status, 200, "Create index should succeed");

    // Query by index (the index is empty since no keys have the field)
    let body: serde_json::Value =
        test_helpers::get_json(&mut app, "/keys/by-index?index=status&eq=active").await;
    assert!(
        body.get("error").is_none(),
        "Index query should not error: {:?}",
        body
    );
    let records = body["data"]["records"].as_array().unwrap();
    assert!(
        records.is_empty(),
        "Index query should return empty initially"
    );
}

#[actix_web::test]
async fn test_query_index_missing_params() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let app = test_helpers::test_app(engine).await;

    // Missing `index` parameter -> should return 400
    let req = actix_web::test::TestRequest::get()
        .uri("/keys/by-index?eq=active")
        .to_request();
    let resp = actix_web::test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        400,
        "Missing index param should return 400"
    );

    // Missing `eq` parameter -> should return 400
    let req = actix_web::test::TestRequest::get()
        .uri("/keys/by-index?index=status")
        .to_request();
    let resp = actix_web::test::call_service(&app, req).await;
    assert_eq!(
        resp.status(),
        400,
        "Missing eq param should return 400"
    );
}
