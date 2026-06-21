//! HTTP API integration tests for the transaction endpoints.
//!
//! Covers:
//! - Begin transaction (`POST /txn`)
//! - Staging a put (`POST /txn/{id}/put`)
//! - Committing (`POST /txn/{id}/commit`)
//! - Rolling back (`POST /txn/{id}/rollback`)
//! - Multiple keys within one transaction
//! - Commit of an empty transaction
//! - Staging a delete within a transaction

use apexstore::api::test_helpers;
use serde_json::json;

#[actix_web::test]
async fn test_begin_transaction() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    let (status, body) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    assert_eq!(status, 200, "POST /txn should return 200 OK");
    assert!(
        body.get("txn_id").is_some(),
        "Response should contain 'txn_id'"
    );
    let txn_id = body["txn_id"].as_u64().expect("txn_id should be a u64");
    assert!(txn_id > 0, "txn_id should be positive");
}

#[actix_web::test]
async fn test_transaction_put_and_commit() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    // Begin a transaction
    let (_, txn_resp) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    let txn_id = txn_resp["txn_id"].as_u64().expect("valid txn_id");

    // Stage a put
    let (status, _body) = test_helpers::post_json(
        &mut app,
        &format!("/txn/{}/put", txn_id),
        &json!({"key": "txn-key1", "value": "txn-val1"}),
    )
    .await;
    assert_eq!(status, 200, "PUT inside transaction should succeed");

    // Verify the engine does NOT have the key yet (uncommitted)
    let val = engine.get("txn-key1").expect("engine get should succeed");
    assert!(val.is_none(), "Key should NOT be visible before commit");

    // Commit
    let (status, commit_body) =
        test_helpers::post_json(&mut app, &format!("/txn/{}/commit", txn_id), &json!({})).await;
    assert_eq!(status, 200, "Commit should succeed");
    assert_eq!(commit_body["status"], "ok");

    // Verify the engine now has the key
    let val = engine.get("txn-key1").expect("engine get should succeed");
    assert_eq!(
        val,
        Some("txn-val1".as_bytes().to_vec()),
        "Key should be visible after commit"
    );
}

#[actix_web::test]
async fn test_transaction_rollback() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    // Begin a transaction
    let (_, txn_resp) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    let txn_id = txn_resp["txn_id"].as_u64().expect("valid txn_id");

    // Stage a put
    test_helpers::post_json(
        &mut app,
        &format!("/txn/{}/put", txn_id),
        &json!({"key": "rollback-key", "value": "rollback-val"}),
    )
    .await;

    // Rollback
    let (status, rollback_body) =
        test_helpers::post_json(&mut app, &format!("/txn/{}/rollback", txn_id), &json!({})).await;
    assert_eq!(status, 200, "Rollback should succeed");
    assert_eq!(rollback_body["status"], "ok");

    // Verify the engine does NOT have the key
    let val = engine
        .get("rollback-key")
        .expect("engine get should succeed");
    assert!(val.is_none(), "Key should NOT exist after rollback");
}

#[actix_web::test]
async fn test_transaction_multiple_keys() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    // Begin a transaction
    let (_, txn_resp) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    let txn_id = txn_resp["txn_id"].as_u64().expect("valid txn_id");

    // Stage multiple puts
    for i in 0..3 {
        let (status, _) = test_helpers::post_json(
            &mut app,
            &format!("/txn/{}/put", txn_id),
            &json!({"key": format!("multi-key-{}", i), "value": format!("multi-val-{}", i)}),
        )
        .await;
        assert_eq!(status, 200, "Put {} should succeed", i);
    }

    // Commit
    test_helpers::post_json(&mut app, &format!("/txn/{}/commit", txn_id), &json!({})).await;

    // Verify all keys exist
    for i in 0..3 {
        let val = engine
            .get(format!("multi-key-{}", i))
            .expect("engine get should succeed");
        assert_eq!(
            val,
            Some(format!("multi-val-{}", i).as_bytes().to_vec()),
            "Key multi-key-{} should have correct value after commit",
            i
        );
    }
}

#[actix_web::test]
async fn test_transaction_commit_empty() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    // Begin a transaction
    let (_, txn_resp) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    let txn_id = txn_resp["txn_id"].as_u64().expect("valid txn_id");

    // Commit without staging any writes
    let (status, commit_body) =
        test_helpers::post_json(&mut app, &format!("/txn/{}/commit", txn_id), &json!({})).await;
    assert_eq!(status, 200, "Empty commit should succeed");
    assert_eq!(commit_body["status"], "ok");
}

#[actix_web::test]
async fn test_transaction_delete() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine.clone()).await;

    // First, directly insert a key
    engine
        .set("delete-me", "will-be-deleted")
        .expect("pre-insert should succeed");

    // Begin a transaction
    let (_, txn_resp) = test_helpers::post_json(&mut app, "/txn", &json!({})).await;
    let txn_id = txn_resp["txn_id"].as_u64().expect("valid txn_id");

    // Stage a delete
    let (status, _body) = test_helpers::post_json(
        &mut app,
        &format!("/txn/{}/delete", txn_id),
        &json!({"key": "delete-me"}),
    )
    .await;
    assert_eq!(status, 200, "Delete inside transaction should succeed");

    // Key should still exist before commit
    let val = engine.get("delete-me").expect("engine get should succeed");
    assert!(val.is_some(), "Key should still exist before commit");

    // Commit
    test_helpers::post_json(&mut app, &format!("/txn/{}/commit", txn_id), &json!({})).await;

    // Key should be deleted after commit
    let val = engine.get("delete-me").expect("engine get should succeed");
    assert!(val.is_none(), "Key should be deleted after commit");
}

#[actix_web::test]
async fn test_transaction_nonexistent_id_returns_404() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = std::sync::Arc::new(engine);
    let mut app = test_helpers::test_app(engine).await;

    // Try to commit a non-existent transaction
    let (status, body) = test_helpers::post_json(&mut app, "/txn/99999/commit", &json!({})).await;
    assert_eq!(status, 404, "Non-existent txn should return 404");
    assert_eq!(body["error"], "transaction not found");

    // Try to rollback a non-existent transaction
    let (status, body) = test_helpers::post_json(&mut app, "/txn/99999/rollback", &json!({})).await;
    assert_eq!(status, 404);
    assert_eq!(body["error"], "transaction not found");
}
