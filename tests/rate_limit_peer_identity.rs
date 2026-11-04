use actix_web::{dev::Service, http::StatusCode, test::TestRequest, web, App, HttpResponse};
use apexstore::api::rate_limiter::{get_client_ip, RateLimiter, RateLimiterState};
use std::net::{IpAddr, SocketAddr};

#[test]
fn client_headers_cannot_replace_or_erase_the_transport_peer() {
    let peer: SocketAddr = "192.0.2.10:1234".parse().unwrap();
    for header in ["198.51.100.1", "198.51.100.1, 10.0.0.1", "invalid", ""] {
        let req = TestRequest::default()
            .peer_addr(peer)
            .insert_header(("x-forwarded-for", header))
            .insert_header(("forwarded", "for=203.0.113.7"))
            .to_srv_request();
        assert_eq!(get_client_ip(&req), Some(peer.ip()), "header: {header}");
    }
    let req = TestRequest::default()
        .peer_addr(peer)
        .insert_header((
            "x-forwarded-for",
            actix_web::http::header::HeaderValue::from_bytes(&[0xff]).unwrap(),
        ))
        .to_srv_request();
    assert_eq!(get_client_ip(&req), Some(peer.ip()));
}

#[actix_web::test]
async fn rotating_forwarded_headers_cannot_bypass_the_real_rate_limiter() {
    let state = web::Data::new(RateLimiterState::new(1));
    let app = actix_web::test::init_service(
        App::new()
            .app_data(state.clone())
            .wrap(RateLimiter)
            .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
    )
    .await;
    let peer: SocketAddr = "192.0.2.10:1234".parse().unwrap();
    let first = TestRequest::get().uri("/").peer_addr(peer).to_request();
    assert_eq!(app.call(first).await.unwrap().status(), StatusCode::OK);
    for n in 1..=100 {
        let req = TestRequest::get()
            .uri("/")
            .peer_addr(peer)
            .insert_header(("x-forwarded-for", format!("198.51.100.{n}")))
            .insert_header(("forwarded", format!("for=203.0.113.{n}")))
            .to_request();
        let rejected = app
            .call(req)
            .await
            .expect_err("forged IP bypassed the limit");
        assert_eq!(
            rejected.as_response_error().status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    let summary = state.get_state();
    assert_eq!(summary.tracked_ips.len(), 1);
    assert_eq!(
        summary.tracked_ips[0].ip.parse::<IpAddr>().unwrap(),
        peer.ip()
    );
    assert_eq!(summary.tracked_ips[0].request_count, 1);
    let other = TestRequest::get()
        .uri("/")
        .peer_addr("192.0.2.11:1234".parse().unwrap())
        .insert_header(("x-forwarded-for", peer.ip().to_string()))
        .to_request();
    assert_eq!(app.call(other).await.unwrap().status(), StatusCode::OK);
    assert_eq!(state.get_state().tracked_ips.len(), 2);
}
