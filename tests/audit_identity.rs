use actix_web::{
    http::{header::HeaderValue, StatusCode},
    test, web, App, HttpResponse,
};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::{
    audit_middleware::AuditMiddleware,
    auth::{request_validator, Permission, TokenManager},
    config::AuthEnabled,
    rate_limiter::{RateLimiter, RateLimiterState},
};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use tracing::{
    field::{Field, Visit},
    instrument::WithSubscriber,
    Event, Subscriber,
};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

#[derive(Clone, Default)]
struct AuditCapture(Arc<Mutex<Vec<Value>>>);
struct Fields(Map<String, Value>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().into(), Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().into(), Value::String(value.into()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), Value::from(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), Value::from(value));
    }
}
impl<S: Subscriber> Layer<S> for AuditCapture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() == "apexstore::audit" {
            let mut fields = Fields(Map::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(Value::Object(fields.0));
        }
    }
}

#[actix_web::test]
async fn audit_records_transport_identity_and_authenticated_principal() {
    let capture = AuditCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    async {
        let manager = TokenManager::new();
        let (token, _) = manager
            .create_token("reader".into(), None, vec![Permission::Read])
            .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(manager))
                .app_data(web::Data::new(AuthEnabled(true)))
                .wrap(AuditMiddleware)
                .wrap(HttpAuthentication::with_fn(request_validator))
                .route("/private", web::get().to(HttpResponse::Ok)),
        )
        .await;
        for (peer, forwarded) in [
            (
                Some("192.0.2.1:1234"),
                HeaderValue::from_static("198.51.100.10"),
            ),
            (
                Some("[2001:db8::1]:1234"),
                HeaderValue::from_bytes(b"\x80").unwrap(),
            ),
            (None, HeaderValue::from_static("198.51.100.10")),
        ] {
            let mut req = test::TestRequest::get()
                .uri("/private")
                .insert_header(("Authorization", format!("Bearer {token}")))
                .insert_header(("X-Forwarded-For", forwarded))
                .insert_header(("Forwarded", "for=198.51.100.20"));
            if let Some(peer) = peer {
                req = req.peer_addr(peer.parse().unwrap());
            }
            assert_eq!(
                test::call_service(&app, req.to_request()).await.status(),
                StatusCode::OK
            );
        }
    }
    .with_subscriber(subscriber)
    .await;
    let events = capture.0.lock().unwrap();
    assert_eq!(events.len(), 3);
    for (event, expected_ip) in events.iter().zip(["192.0.2.1", "2001:db8::1", "unknown"]) {
        assert_eq!(event["client_ip"], expected_ip);
        assert_eq!(event["principal"], "reader");
        assert_eq!(event["status"], 200);
        assert_eq!(event["path"], "/private");
    }
}

#[actix_web::test]
async fn audit_records_the_actual_rate_limit_rejection_status() {
    let capture = AuditCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    async {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(RateLimiterState::new(1)))
                .wrap(RateLimiter)
                .wrap(AuditMiddleware)
                .route("/keys", web::get().to(HttpResponse::Ok)),
        )
        .await;
        let request = || {
            test::TestRequest::get()
                .uri("/keys")
                .peer_addr("192.0.2.9:1234".parse().unwrap())
                .to_request()
        };
        assert_eq!(
            test::call_service(&app, request()).await.status(),
            StatusCode::OK
        );
        let error = test::try_call_service(&app, request()).await.unwrap_err();
        assert_eq!(
            error.as_response_error().status_code(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    .with_subscriber(subscriber)
    .await;
    let events = capture.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["status"], 200);
    assert_eq!(events[1]["status"], 429);
    assert_eq!(events[1]["client_ip"], "192.0.2.9");
    assert_eq!(events[1]["principal"], "anonymous");
}
