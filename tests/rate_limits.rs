use actix_web::{http::StatusCode, test, web, App, HttpResponse};
use apexstore::api::rate_limiter::{RateLimiter, RateLimiterState};

#[actix_web::test]
async fn endpoint_limit_remains_enabled_without_a_global_limit() {
    let mut limiter = RateLimiterState::new(0);
    limiter.set_endpoint_limit("/admin/compact", 2);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(limiter))
            .wrap(RateLimiter)
            .route("/admin/compact", web::post().to(HttpResponse::Ok)),
    )
    .await;
    for expected in [
        StatusCode::OK,
        StatusCode::OK,
        StatusCode::TOO_MANY_REQUESTS,
    ] {
        let request = test::TestRequest::post()
            .uri("/admin/compact")
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .to_request();
        let status = match test::try_call_service(&app, request).await {
            Ok(response) => response.status(),
            Err(error) => error.as_response_error().status_code(),
        };
        assert_eq!(status, expected);
    }
}

#[actix_web::test]
async fn concrete_paths_share_the_registered_route_bucket() {
    let mut limiter = RateLimiterState::new(1000);
    limiter.set_endpoint_limit("/api/keys/{key}", 50);
    let state = web::Data::new(limiter);
    let app = test::init_service(
        App::new()
            .app_data(state.clone())
            .wrap(RateLimiter)
            .service(web::scope("/api").route("/keys/{key}", web::get().to(HttpResponse::Ok))),
    )
    .await;
    for key in 0..50 {
        let request = test::TestRequest::get()
            .uri(&format!("/api/keys/{key}"))
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .to_request();
        assert_eq!(
            test::call_service(&app, request).await.status(),
            StatusCode::OK
        );
    }
    let summary = state.get_state();
    assert_eq!(summary.tracked_ips.len(), 1);
    assert_eq!(summary.tracked_ips[0].request_count, 50);
    assert_eq!(summary.tracked_ips[0].endpoint_counts.len(), 1);
    assert_eq!(
        summary.tracked_ips[0].endpoint_counts["/api/keys/{key}"],
        50
    );
    let request = test::TestRequest::get()
        .uri("/api/keys/another")
        .peer_addr("127.0.0.1:12345".parse().unwrap())
        .to_request();
    let error = test::try_call_service(&app, request).await.unwrap_err();
    assert_eq!(
        error.as_response_error().status_code(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[actix_web::test]
async fn unmatched_paths_share_a_bounded_bucket_and_the_global_limit() {
    let state = web::Data::new(RateLimiterState::new(3));
    let app = test::init_service(App::new().app_data(state.clone()).wrap(RateLimiter)).await;
    for key in 0..3 {
        let request = test::TestRequest::get()
            .uri(&format!("/missing/{key}"))
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .to_request();
        assert_eq!(
            test::call_service(&app, request).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    let summary = state.get_state();
    assert_eq!(summary.tracked_ips[0].endpoint_counts.len(), 1);
    assert_eq!(summary.tracked_ips[0].endpoint_counts["<unmatched>"], 3);
    let request = test::TestRequest::get()
        .uri("/yet-another-missing-path")
        .peer_addr("127.0.0.1:12345".parse().unwrap())
        .to_request();
    let error = test::try_call_service(&app, request).await.unwrap_err();
    assert_eq!(
        error.as_response_error().status_code(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
