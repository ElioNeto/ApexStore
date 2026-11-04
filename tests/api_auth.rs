use actix_web::{http::StatusCode, test, web, App};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::auth::{request_validator, require_permission, Permission, TokenManager};
use apexstore::api::config::{AccessControlEnabled, AuthEnabled};
use apexstore::api::configure;
use apexstore::api::test_helpers::test_engine;

#[actix_web::test]
async fn authentication_is_independent_of_access_control_flag() {
    for access_control_enabled in [false, true] {
        let (engine, _dir) = test_engine();
        let manager = TokenManager::new();
        let (reader, _) = manager
            .create_token("reader".into(), None, vec![Permission::Read])
            .expect("create reader token");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(engine))
                .app_data(web::Data::new(manager))
                .app_data(web::Data::new(AuthEnabled(true)))
                .app_data(web::Data::new(AccessControlEnabled(access_control_enabled)))
                .wrap(HttpAuthentication::with_fn(request_validator))
                .configure(configure),
        )
        .await;

        for authorization in [None, Some("Bearer invalido")] {
            let mut request = test::TestRequest::get().uri("/keys/x");
            if let Some(value) = authorization {
                request = request.insert_header(("Authorization", value));
            }
            let response = test::call_service(&app, request.to_request()).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/keys/x")
                .insert_header(("Authorization", format!("Bearer {reader}")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        for request in [
            test::TestRequest::put()
                .uri("/keys/x")
                .set_json(serde_json::json!({"value": "blocked"})),
            test::TestRequest::post().uri("/admin/flush"),
        ] {
            let response = test::call_service(
                &app,
                request
                    .insert_header(("Authorization", format!("Bearer {reader}")))
                    .to_request(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }
}

#[actix_web::test]
async fn missing_authentication_configuration_requires_valid_token() {
    let manager = TokenManager::new();
    let (reader, _) = manager
        .create_token("reader".into(), None, vec![Permission::Read])
        .expect("create reader token");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(manager))
            .app_data(web::Data::new(AccessControlEnabled(false)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .route("/", web::get().to(|| async { "authenticated" })),
    )
    .await;
    for (token, expected) in [
        ("invalido".to_string(), StatusCode::UNAUTHORIZED),
        (reader, StatusCode::OK),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), expected);
    }
    let request = test::TestRequest::default().to_http_request();
    assert!(require_permission(&request, Permission::Read).is_err());
}

#[actix_web::test]
async fn authentication_can_be_explicitly_disabled() {
    let request = test::TestRequest::default()
        .app_data(web::Data::new(AuthEnabled(false)))
        .app_data(web::Data::new(AccessControlEnabled(true)))
        .to_http_request();
    assert!(require_permission(&request, Permission::Admin).is_ok());
}

#[actix_web::test]
async fn disabled_auth_accepts_requests_without_bearer_credentials() {
    let (engine, _dir) = test_engine();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(engine))
            .app_data(web::Data::new(AuthEnabled(false)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/keys/container-persistence")
            .set_json(serde_json::json!({"value":"survives-recreation"}))
            .to_request(),
    )
    .await;
    assert!(response.status().is_success());
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/keys/container-persistence")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(response).await;
    assert_eq!(body["value"], "survives-recreation");
}

#[actix_web::test]
async fn only_exact_liveness_probe_is_public() {
    for enabled in [None, Some(true)] {
        let (engine, _dir) = test_engine();
        let mut app = App::new().app_data(web::Data::new(engine));
        if let Some(enabled) = enabled {
            app = app.app_data(web::Data::new(AuthEnabled(enabled)));
        }
        let app = test::init_service(
            app.wrap(HttpAuthentication::with_fn(request_validator))
                .configure(configure),
        )
        .await;
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/health/liveness")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        for path in [
            "/keys/x",
            "/admin/flush",
            "/health/readiness",
            "/health/startup",
            "/health/check",
            "/health/liveness/",
            "/health/liveness-extra",
        ] {
            let response =
                test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/health/liveness")
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
