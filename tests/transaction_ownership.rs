use actix_web::{http::StatusCode, test, web, App};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::{
    auth::request_validator, config::AuthEnabled, configure, test_helpers::test_engine, Permission,
    TokenManager, TransactionManager,
};
use serde_json::{json, Value};
use std::sync::Arc;

async fn rejects_foreign_transaction(operation: &str) {
    let (engine, _directory) = test_engine();
    let engine = Arc::new(engine);
    engine.set("retained", b"original").unwrap();
    let tokens = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (owner, _) = tokens
        .create_token("owner".into(), None, vec![Permission::Admin])
        .unwrap();
    // Even another administrator cannot operate on a transaction it does not own.
    let (stranger, _) = tokens
        .create_token("stranger".into(), None, vec![Permission::Admin])
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(engine.clone()))
            .app_data(web::Data::new(tokens))
            .app_data(web::Data::new(TransactionManager::new()))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let request = |path: &str, token: &str, body: Value| {
        test::TestRequest::post()
            .uri(path)
            .insert_header(("Authorization", format!("Bearer {token}")))
            .set_json(body)
            .to_request()
    };
    let response: Value =
        test::call_and_read_body_json(&app, request("/txn", &owner, json!({}))).await;
    let id = response["txn_id"].as_str().expect("UUID string");
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 4);
    let response = test::call_service(
        &app,
        request(
            &format!("/txn/{id}/put"),
            &owner,
            json!({"key":"owned", "value":"staged"}),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = test::call_service(
        &app,
        request(
            &format!("/txn/{id}/{operation}"),
            &stranger,
            json!({"key":"retained", "value":"injected"}),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "{operation}");
    let foreign_body: Value = test::read_body_json(response).await;
    let response = test::call_service(
        &app,
        request(
            &format!("/txn/00000000-0000-4000-8000-000000000000/{operation}"),
            &stranger,
            json!({"key":"retained", "value":"injected"}),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        foreign_body,
        test::read_body_json::<Value, _>(response).await
    );
    assert_eq!(engine.get("owned").unwrap(), None);
    let response = test::call_service(
        &app,
        request(&format!("/txn/{id}/commit"), &owner, json!({})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(engine.get("owned").unwrap(), Some(b"staged".to_vec()));
    assert_eq!(engine.get("retained").unwrap(), Some(b"original".to_vec()));
}

#[actix_web::test]
async fn foreign_put_is_hidden_and_preserves_transaction() {
    rejects_foreign_transaction("put").await;
}

#[actix_web::test]
async fn foreign_delete_is_hidden_and_preserves_transaction() {
    rejects_foreign_transaction("delete").await;
}

#[actix_web::test]
async fn foreign_commit_is_hidden_and_preserves_transaction() {
    rejects_foreign_transaction("commit").await;
}

#[actix_web::test]
async fn foreign_rollback_is_hidden_and_preserves_transaction() {
    rejects_foreign_transaction("rollback").await;
}

#[actix_web::test]
async fn capacity_is_per_owner_and_global_and_released_by_rollback() {
    let (engine, _directory) = test_engine();
    let engine = Arc::new(engine);
    let tokens = TokenManager::new_with_engine(engine.clone()).unwrap();
    let owners: Vec<_> = (0..3)
        .map(|n| {
            tokens
                .create_token(n.to_string(), None, vec![Permission::Admin])
                .unwrap()
                .0
        })
        .collect();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(engine))
            .app_data(web::Data::new(tokens))
            .app_data(web::Data::new(TransactionManager::with_limits(
                std::time::Duration::from_secs(300),
                1,
                2,
            )))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let request = |path: &str, index: usize| {
        test::TestRequest::post()
            .uri(path)
            .insert_header(("Authorization", format!("Bearer {}", owners[index])))
            .to_request()
    };
    let first: Value = test::call_and_read_body_json(&app, request("/txn", 0)).await;
    assert_eq!(
        test::call_service(&app, request("/txn", 0)).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let second: Value = test::call_and_read_body_json(&app, request("/txn", 1)).await;
    assert_ne!(first["txn_id"], second["txn_id"]);
    assert_eq!(
        test::call_service(&app, request("/txn", 2)).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let path = format!("/txn/{}/rollback", first["txn_id"].as_str().unwrap());
    assert_eq!(
        test::call_service(&app, request(&path, 0)).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        test::call_service(&app, request("/txn", 2)).await.status(),
        StatusCode::OK
    );
    let path = format!("/txn/{}/commit", second["txn_id"].as_str().unwrap());
    assert_eq!(
        test::call_service(&app, request(&path, 1)).await.status(),
        StatusCode::OK
    );
}

#[actix_web::test]
async fn expired_transactions_cannot_be_used_and_free_capacity_without_committing() {
    let (engine, _directory) = test_engine();
    let engine = Arc::new(engine);
    let tokens = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (raw, _) = tokens
        .create_token("owner".into(), None, vec![Permission::Admin])
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(engine.clone()))
            .app_data(web::Data::new(tokens))
            .app_data(web::Data::new(TransactionManager::with_limits(
                std::time::Duration::from_secs(1),
                1,
                1,
            )))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let request = |path: &str| {
        test::TestRequest::post()
            .uri(path)
            .insert_header(("Authorization", format!("Bearer {raw}")))
            .set_json(json!({"key":"expired", "value":"never-commit"}))
            .to_request()
    };
    let response: Value = test::call_and_read_body_json(&app, request("/txn")).await;
    let id = response["txn_id"].as_str().unwrap();
    assert_eq!(
        test::call_service(&app, request(&format!("/txn/{id}/put")))
            .await
            .status(),
        StatusCode::OK
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    for operation in ["put", "delete", "commit", "rollback"] {
        let response = test::call_service(&app, request(&format!("/txn/{id}/{operation}"))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{operation}");
    }
    assert_eq!(engine.get("expired").unwrap(), None);
    assert_eq!(
        test::call_service(&app, request("/txn")).await.status(),
        StatusCode::OK
    );
}

#[actix_web::test]
async fn owner_can_delete_and_rollback_and_revocation_still_requires_authentication() {
    let (engine, _directory) = test_engine();
    let engine = Arc::new(engine);
    engine.set("retained", b"value").unwrap();
    let tokens = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (raw, token) = tokens
        .create_token("owner".into(), None, vec![Permission::Admin])
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(engine.clone()))
            .app_data(web::Data::new(tokens.clone()))
            .app_data(web::Data::new(TransactionManager::new()))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let request = |path: &str| {
        test::TestRequest::post()
            .uri(path)
            .insert_header(("Authorization", format!("Bearer {raw}")))
            .set_json(json!({"key":"retained"}))
            .to_request()
    };
    for outcome in ["rollback", "commit"] {
        let response: Value = test::call_and_read_body_json(&app, request("/txn")).await;
        let id = response["txn_id"].as_str().unwrap();
        assert_eq!(
            test::call_service(&app, request(&format!("/txn/{id}/delete")))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(engine.get("retained").unwrap(), Some(b"value".to_vec()));
        assert_eq!(
            test::call_service(&app, request(&format!("/txn/{id}/{outcome}")))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            engine.get("retained").unwrap().is_some(),
            outcome == "rollback"
        );
    }
    let response: Value = test::call_and_read_body_json(&app, request("/txn")).await;
    let id = response["txn_id"].as_str().unwrap();
    tokens.delete_token(&token.id).unwrap();
    assert_eq!(
        test::call_service(&app, request(&format!("/txn/{id}/commit")))
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}
