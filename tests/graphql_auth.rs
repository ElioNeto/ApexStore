use actix_web::{http::StatusCode, test, web, App};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::auth::{request_validator, Permission, TokenManager};
use apexstore::api::config::AuthEnabled;
use apexstore::api::{configure, graphql::build_schema, test_helpers::test_engine};
use apexstore::core::engine::MAX_SCAN_LIMIT;
use serde_json::json;

#[actix_web::test]
async fn graphql_enforces_each_field_permission() {
    let (engine, _dir) = test_engine();
    let engine = std::sync::Arc::new(engine);
    let manager = TokenManager::new();
    let roles = [
        ("none", vec![]),
        ("reader", vec![Permission::Read]),
        ("writer", vec![Permission::Write]),
        ("deleter", vec![Permission::Delete]),
        ("admin", vec![Permission::Admin]),
    ];
    let tokens: Vec<_> = roles
        .into_iter()
        .map(|(name, permissions)| {
            let (raw, token) = manager
                .create_token(name.into(), None, permissions)
                .unwrap();
            (raw, token)
        })
        .collect();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(build_schema(engine.clone())))
            .app_data(web::Data::new(manager))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;

    for (raw, token) in tokens {
        for (query, permission) in [
            (r#"{ get(key: "protected") }"#, Permission::Read),
            ("{ scan { key value } }", Permission::Read),
            ("{ keys }", Permission::Read),
            ("{ stats { totalRecords } }", Permission::Read),
            (
                r#"mutation { set(key: "protected", value: "changed") }"#,
                Permission::Write,
            ),
            (
                r#"mutation { delete(key: "protected") }"#,
                Permission::Delete,
            ),
        ] {
            engine.set("protected", b"original").unwrap();
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/graphql")
                    .insert_header(("Authorization", format!("Bearer {raw}")))
                    .set_json(json!({"query": query}))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body: serde_json::Value = test::read_body_json(response).await;
            let allowed = token.has_permission(permission);
            assert_eq!(
                body.get("errors").is_none(),
                allowed,
                "{}: {query}: {body}",
                token.name
            );
            if !allowed {
                assert_eq!(body["errors"][0]["extensions"]["code"], "FORBIDDEN");
                assert_eq!(engine.get("protected").unwrap(), Some(b"original".to_vec()));
            }
        }
    }
}

#[actix_web::test]
async fn graphql_guards_apply_to_aliases_fragments_and_selected_operations() {
    let (engine, _dir) = test_engine();
    let engine = std::sync::Arc::new(engine);
    let manager = TokenManager::new();
    let (reader, _) = manager
        .create_token("reader".into(), None, vec![Permission::Read])
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(build_schema(engine.clone())))
            .app_data(web::Data::new(manager))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let response = test::call_service(&app, test::TestRequest::post().uri("/graphql")
        .insert_header(("Authorization", format!("Bearer {reader}")))
        .set_json(json!({
            "query": "query Harmless { keys } mutation Attack { ...WriteFields } fragment WriteFields on Mutation { disguised: set(key: \"forbidden\", value: \"x\") }",
            "operationName": "Attack"
        })).to_request()).await;
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["errors"][0]["extensions"]["code"], "FORBIDDEN");
    assert_eq!(engine.get("forbidden").unwrap(), None);
}

#[actix_web::test]
async fn graphql_caps_scan_and_keys_results() {
    let (engine, _dir) = test_engine();
    let engine = std::sync::Arc::new(engine);
    for i in 0..MAX_SCAN_LIMIT + 3 {
        engine.set(format!("key_{i:05}"), b"value").unwrap();
    }
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(build_schema(engine)))
            .app_data(web::Data::new(AuthEnabled(false)))
            .configure(configure),
    )
    .await;
    for query in ["{ scan(limit: 2147483647) { key value } }", "{ keys }"] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/graphql")
                .set_json(json!({"query": query}))
                .to_request(),
        )
        .await;
        let body: serde_json::Value = test::read_body_json(response).await;
        assert!(body.get("errors").is_none(), "{body}");
        let field = if query.contains("scan") {
            "scan"
        } else {
            "keys"
        };
        assert_eq!(
            body["data"][field].as_array().unwrap().len(),
            MAX_SCAN_LIMIT
        );
    }
}

#[actix_web::test]
async fn graphql_rejects_excessive_depth_and_complexity_before_writing() {
    let (engine, _dir) = test_engine();
    let engine = std::sync::Arc::new(engine);
    let schema = build_schema(engine.clone());
    let query = format!(
        "mutation {{ {} }}",
        (0..201)
            .map(|i| format!("a{i}: set(key: \"key_{i}\", value: \"x\")"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let result = schema
        .execute(async_graphql::Request::new(query).data(AuthEnabled(false)))
        .await;
    assert!(
        !result.errors.is_empty(),
        "excessive complexity must be rejected"
    );
    assert_eq!(result.errors[0].message, "Query is too complex.");
    assert!(
        engine.keys().unwrap().is_empty(),
        "rejection must precede every mutation"
    );
    let query = format!(
        "{{ __schema {{ types {{ {} name {} }} }} }}",
        "ofType { ".repeat(9),
        " }".repeat(9)
    );
    let result = schema
        .execute(async_graphql::Request::new(query).data(AuthEnabled(false)))
        .await;
    assert!(
        !result.errors.is_empty(),
        "excessive depth must be rejected"
    );
    assert_eq!(result.errors[0].message, "Query is nested too deep.");
}

#[actix_web::test]
async fn graphql_requires_auth_context_unless_explicitly_disabled() {
    let (engine, _dir) = test_engine();
    let engine = std::sync::Arc::new(engine);
    let schema = build_schema(engine.clone());
    let query = r#"mutation { set(key: "protected", value: "x") }"#;
    let result = schema.execute(query).await;
    assert!(!result.errors.is_empty());
    assert_eq!(engine.get("protected").unwrap(), None);
    let result = schema
        .execute(async_graphql::Request::new(query).data(AuthEnabled(false)))
        .await;
    assert!(result.errors.is_empty());
    assert_eq!(engine.get("protected").unwrap(), Some(b"x".to_vec()));
}
