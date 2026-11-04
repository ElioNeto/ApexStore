use actix_web::{web, App, HttpResponse, HttpServer};
use apexstore::api::{config::AuthEnabled, configure, test_helpers};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[actix_web::test]
async fn http2_serves_real_key_value_routes() {
    let (engine, _dir) = test_helpers::test_engine();
    let engine = web::Data::new(engine);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(engine.clone())
            .app_data(web::Data::new(AuthEnabled(false)))
            .configure(configure)
    })
    .workers(1)
    .listen_auto_h2c(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let task = actix_web::rt::spawn(server);
    let client = reqwest::Client::builder()
        .http2_prior_knowledge()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let value = "body-over-http2".repeat(2_000);
    let url = format!("http://{addr}/keys/h2-key");
    let put = client
        .put(&url)
        .json(&serde_json::json!({"value": value}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.version(), reqwest::Version::HTTP_2);
    assert!(put.status().is_success());
    put.text().await.unwrap();
    let get = client.get(&url).send().await.unwrap();
    assert_eq!(get.version(), reqwest::Version::HTTP_2);
    assert!(get.status().is_success());
    let json: serde_json::Value = get.json().await.unwrap();
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), handle.stop(true))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(json["value"], value);
}

#[actix_web::test]
async fn http2_rejects_empty_data_frame_flood() {
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let ready = started.clone();
    let done = release.clone();
    let server = HttpServer::new(move || {
        let ready = ready.clone();
        let done = done.clone();
        App::new().route(
            "/",
            web::post().to(move |payload: web::Payload| {
                let ready = ready.clone();
                let done = done.clone();
                async move {
                    ready.notify_one();
                    // Deliberately hold the body without draining it, as in the advisory.
                    done.notified().await;
                    drop(payload);
                    HttpResponse::Ok().finish()
                }
            }),
        )
    })
    .workers(1)
    .listen_auto_h2c(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    let task = actix_web::rt::spawn(server);
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n")
        .await
        .unwrap();
    socket
        .write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0])
        .await
        .unwrap(); // SETTINGS
                   // HPACK: :method POST, :scheme http, :path /, :authority localhost.
    let headers = b"\x83\x86\x84\x01\x09localhost";
    socket
        .write_all(&[0, 0, headers.len() as u8, 1, 4, 0, 0, 0, 1])
        .await
        .unwrap();
    socket.write_all(headers).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();
    let (mut reader, mut writer) = socket.into_split();
    let flood = [0, 0, 0, 0, 0, 0, 0, 0, 1].repeat(100_000);
    let read_goaway = async {
        loop {
            let mut header = [0; 9];
            reader.read_exact(&mut header).await?;
            let len = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
            assert!(len <= 65_536);
            let mut payload = vec![0; len];
            reader.read_exact(&mut payload).await?;
            if header[3] == 7 {
                assert!(payload.len() >= 8);
                return Ok::<_, std::io::Error>(u32::from_be_bytes(
                    payload[4..8].try_into().unwrap(),
                ));
            }
        }
    };
    let (goaway, _write_result) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(10), read_goaway),
        tokio::time::timeout(Duration::from_secs(10), writer.write_all(&flood)),
    );
    release.notify_one();
    handle.stop(false).await;
    task.await.unwrap().unwrap();
    assert!(
        matches!(goaway, Ok(Ok(11))),
        "expected GOAWAY/ENHANCE_YOUR_CALM, got {goaway:?}"
    );
}
