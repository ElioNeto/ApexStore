use actix_web::{http::StatusCode, test, web, App, HttpResponse};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::{
    auth::{request_validator, Permission, TokenManager},
    config::AuthEnabled,
    configure,
    connection_guard::IpConnectionGuard,
    rate_limiter::{RateLimiter, RateLimiterState},
};
use std::{
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[actix_web::test]
async fn distinct_transport_peers_cannot_grow_tracking_or_reset_live_budgets() {
    let state = web::Data::new(RateLimiterState::with_capacity(1, 1));
    let app = test::init_service(
        App::new()
            .app_data(state.clone())
            .wrap(RateLimiter)
            .route("/keys/{key}", web::get().to(HttpResponse::Ok)),
    )
    .await;
    let mut admitted = Vec::new();
    let mut rejected = 0;
    for i in 1..=512u16 {
        let peer = format!("192.0.{}.{}:1234", i / 256, i % 256);
        let req = test::TestRequest::get()
            .uri(&format!("/keys/{i}"))
            .peer_addr(peer.parse().unwrap())
            .to_request();
        match test::try_call_service(&app, req).await {
            Ok(response) => {
                assert_eq!(response.status(), StatusCode::OK);
                admitted.push(peer);
            }
            Err(error) => {
                assert_eq!(
                    error.as_response_error().status_code(),
                    StatusCode::TOO_MANY_REQUESTS
                );
                rejected += 1;
            }
        }
    }
    assert!(!admitted.is_empty());
    assert!(admitted.len() <= 16);
    assert!(rejected > 0);
    for peer in &admitted {
        let error = test::try_call_service(
            &app,
            test::TestRequest::get()
                .uri("/keys/retry")
                .peer_addr(peer.parse().unwrap())
                .to_request(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.as_response_error().status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    let snapshot = state.get_state();
    assert_eq!(snapshot.tracked_ip_count, admitted.len());
    assert_eq!(snapshot.capacity_rejections, rejected);
    assert!(snapshot
        .tracked_ips
        .iter()
        .all(|ip| ip.request_count == 1 && ip.endpoint_counts.len() == 1));
}

#[actix_web::test]
async fn tracking_pressure_metrics_remain_admin_only() {
    let manager = TokenManager::new();
    let (reader, _) = manager
        .create_token("reader".into(), None, vec![Permission::Read])
        .unwrap();
    let (admin, _) = manager
        .create_token("admin".into(), None, vec![Permission::Admin])
        .unwrap();
    let guard = web::Data::new(IpConnectionGuard::with_capacity(2, 1));
    assert!(guard.try_acquire("192.0.2.1"));
    assert!(!guard.try_acquire("192.0.2.2"));
    let state = web::Data::new(RateLimiterState::with_capacity(10, 3));
    let app = test::init_service(
        App::new()
            .app_data(guard)
            .app_data(state)
            .app_data(web::Data::new(manager))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(reader), StatusCode::FORBIDDEN),
        (Some(admin), StatusCode::OK),
    ] {
        let mut req = test::TestRequest::get().uri("/admin/rate_limits");
        if let Some(token) = token {
            req = req.insert_header(("Authorization", format!("Bearer {token}")));
        }
        let response = test::call_service(&app, req.to_request()).await;
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            let body: serde_json::Value = test::read_body_json(response).await;
            assert_eq!(body["global_limit"], 10);
            assert_eq!(body["max_ips_per_shard"], 3);
            assert_eq!(body["shard_count"], 16);
            assert_eq!(body["tracked_ip_count"], 0);
            assert_eq!(body["capacity_rejections"], 0);
            assert_eq!(body["connections"]["tracked_ip_count"], 1);
            assert_eq!(body["connections"]["max_tracked_ips"], 1);
            assert_eq!(body["connections"]["active_requests"], 1);
            assert_eq!(body["connections"]["capacity_rejections"], 1);
        }
    }
}

struct RunningServer(Child);
impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn server_enforces_tracking_capacities_from_environment() {
    for (rate_capacity, connection_capacity, expected) in [
        (0, 2, reqwest::StatusCode::TOO_MANY_REQUESTS),
        (2, 0, reqwest::StatusCode::TOO_MANY_REQUESTS),
        (2, 3, reqwest::StatusCode::OK),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut server = RunningServer(
            Command::new(env!("CARGO_BIN_EXE_apexstore-server"))
                .current_dir(dir.path())
                .env("DATA_DIR", dir.path().join("database"))
                .env("HOST", "127.0.0.1")
                .env("PORT", port.to_string())
                .env("WORKERS", "1")
                .env("ENCRYPTION_ENABLED", "false")
                .env_remove("ENCRYPTION_KEY_PATH")
                .env("API_AUTH_ENABLED", "false")
                .env("ACCESS_CONTROL_ENABLED", "false")
                .env("TLS_ENABLED", "false")
                .env("RATE_LIMIT_ENABLED", "true")
                .env("RATE_LIMIT_REQUESTS_PER_MINUTE", "100")
                .env("MAX_CONNECTIONS_PER_IP", "10")
                .env("RATE_LIMIT_MAX_IPS_PER_SHARD", rate_capacity.to_string())
                .env(
                    "CONNECTION_MAX_TRACKED_IPS",
                    connection_capacity.to_string(),
                )
                .env_remove("CDC_ENDPOINT")
                .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let url = format!("http://127.0.0.1:{port}/admin/rate_limits");
        let deadline = Instant::now() + Duration::from_secs(20);
        let response = loop {
            if let Ok(response) = client.get(&url).send().await {
                break response;
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "server exited before listening"
            );
            assert!(Instant::now() < deadline, "server did not listen");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert_eq!(
            response.status(),
            expected,
            "rate capacity {rate_capacity}, connection capacity {connection_capacity}"
        );
        if expected == reqwest::StatusCode::OK {
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["max_ips_per_shard"], rate_capacity);
            assert_eq!(body["connections"]["max_tracked_ips"], connection_capacity);
        }
    }
}
