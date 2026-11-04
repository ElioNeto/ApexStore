use actix_web::{http::header, test};
use apexstore::api::test_helpers;
use std::sync::Arc;

#[actix_web::test]
async fn dashboard_renders_column_family_names_as_text() {
    let (engine, _dir) = test_helpers::test_engine();
    let malicious_cf = "<img src=x onerror=alert(1)>";
    let special_cf = "quotes\"'& café";
    for cf in [malicious_cf, special_cf] {
        engine
            .put_cf(cf, b"key".to_vec(), b"value".to_vec())
            .expect("write column family");
        engine.flush_memtable_cf(cf).expect("flush column family");
    }
    let app = test_helpers::test_app(Arc::new(engine)).await;
    let request = test::TestRequest::get()
        .uri("/admin/dashboard")
        .to_request();
    let response = test::call_service(&app, request).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
    let html = String::from_utf8(test::read_body(response).await.to_vec()).unwrap();
    assert!(html.contains("<li>&lt;img src=x onerror=alert(1)&gt;</li>"));
    assert!(html.contains("<li>quotes&quot;&#39;&amp; café</li>"));
    assert!(!html.contains(malicious_cf));
    assert!(!html.contains("innerHTML"));
    assert!(html.contains("li.textContent = item.textContent"));
}
