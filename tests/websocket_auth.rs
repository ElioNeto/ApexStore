use actix_web::{web, App, HttpServer};
use actix_web_httpauth::middleware::HttpAuthentication;
use apexstore::api::auth::{request_validator, Permission, TokenManager};
use apexstore::api::config::{AccessControlEnabled, AuthEnabled};
use apexstore::api::sync::{sync_handler, SyncManager};
use apexstore::infra::access_control::{AccessController, AccessPolicy, Effect, Operation};
use apexstore::storage::cache::GlobalBlockCache;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Server {
    address: std::net::SocketAddr,
    handle: actix_web::dev::ServerHandle,
    engine: web::Data<apexstore::LsmEngine>,
    tokens: web::Data<TokenManager>,
    sync: web::Data<SyncManager>,
    directory: tempfile::TempDir,
}

impl Server {
    async fn start() -> Self {
        Self::with_access(None).await
    }

    async fn with_access(controller: Option<AccessController>) -> Self {
        let access_enabled = controller.is_some();
        let controller = web::Data::new(controller.unwrap_or_default());
        let directory = tempfile::tempdir().unwrap();
        let config = apexstore::LsmConfig::builder()
            .dir_path(directory.path().to_path_buf())
            .memtable_max_size(1024)
            .build()
            .unwrap();
        let engine = web::Data::new(
            apexstore::LsmEngine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap(),
        );
        let tokens = web::Data::new(TokenManager::new());
        let sync = web::Data::new(SyncManager::new());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let app_engine = engine.clone();
        let app_tokens = tokens.clone();
        let app_sync = sync.clone();
        let server = HttpServer::new(move || {
            App::new()
                .app_data(app_engine.clone())
                .app_data(app_tokens.clone())
                .app_data(app_sync.clone())
                .app_data(web::Data::new(AuthEnabled(true)))
                .app_data(web::Data::new(AccessControlEnabled(access_enabled)))
                .app_data(controller.clone())
                .wrap(HttpAuthentication::with_fn(request_validator))
                .service(sync_handler)
        })
        .workers(1)
        .listen(listener)
        .unwrap()
        .run();
        let handle = server.handle();
        actix_rt::spawn(server);
        Self {
            address,
            handle,
            engine,
            tokens,
            sync,
            directory,
        }
    }

    async fn connect(&self, token: &str) -> (TcpStream, String) {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        let request = format!(
            "GET /ws/sync HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAuthorization: Bearer {token}\r\n\r\n",
            self.address
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut header = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
                assert!(header.len() < 8192);
            }
        })
        .await
        .unwrap();
        (stream, String::from_utf8(header).unwrap())
    }
}

async fn send(stream: &mut TcpStream, value: Value) {
    let payload = serde_json::to_vec(&value).unwrap();
    let mask = [0x12, 0x34, 0x56, 0x78];
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(u16::try_from(payload.len()).unwrap()).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    stream.write_all(&frame).await.unwrap();
}

async fn receive(stream: &mut TcpStream) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        assert_eq!(
            stream.read_u8().await.unwrap(),
            0x81,
            "expected a final text frame"
        );
        let length = stream.read_u8().await.unwrap();
        assert_eq!(length & 0x80, 0, "server frames must not be masked");
        let length = match length {
            126 => usize::from(stream.read_u16().await.unwrap()),
            127 => usize::try_from(stream.read_u64().await.unwrap()).unwrap(),
            length => usize::from(length),
        };
        assert!(length <= 65536);
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await.unwrap();
        serde_json::from_slice(&payload).unwrap()
    })
    .await
    .unwrap()
}

fn push(key: &str, value: &str, timestamp: u64) -> Value {
    json!({"type":"sync_push", "changes":[{"key":key,"value":value,"timestamp":timestamp,"device_id":"integration-test"}],"last_ack":0})
}

#[actix_web::test]
async fn read_only_token_cannot_upgrade_to_write_channel() {
    let server = Server::start().await;
    let (raw, _) = server
        .tokens
        .create_token("reader".into(), None, vec![Permission::Read])
        .unwrap();
    let (stream, response) = server.connect(&raw).await;
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(5), server.handle.stop(true))
        .await
        .unwrap();
}

#[actix_web::test]
async fn write_token_can_sync_without_receiving_read_protected_data() {
    let server = Server::start().await;
    let (raw, _) = server
        .tokens
        .create_token("writer".into(), None, vec![Permission::Write])
        .unwrap();
    let (mut stream, response) = server.connect(&raw).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    send(&mut stream, push("notes/test", "persisted", 1)).await;
    let response = receive(&mut stream).await;
    assert_eq!(response["type"], "sync_ack");
    assert_eq!(response["changes"], json!([]));
    assert_eq!(
        server.engine.get("notes/test").unwrap(),
        Some(b"persisted".to_vec())
    );
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(5), server.handle.stop(true))
        .await
        .unwrap();
}

#[actix_web::test]
async fn reserved_keys_and_revoked_tokens_cannot_write() {
    let server = Server::start().await;
    let (raw, token) = server
        .tokens
        .create_token("admin".into(), None, vec![Permission::Admin])
        .unwrap();
    let (mut stream, response) = server.connect(&raw).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    for key in ["__token:forged", "__ttl:note", "__idx:__schema:index:kind"] {
        send(&mut stream, push(key, "forged", 1)).await;
        assert_eq!(receive(&mut stream).await["type"], "error");
        assert_eq!(server.engine.get(key).unwrap(), None);
    }
    server.tokens.delete_token(&token.id).unwrap();
    send(&mut stream, push("notes/revoked", "forged", 2)).await;
    assert_eq!(receive(&mut stream).await["type"], "error");
    assert_eq!(server.engine.get("notes/revoked").unwrap(), None);
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(5), server.handle.stop(true))
        .await
        .unwrap();
}

#[actix_web::test]
async fn engine_write_failure_returns_an_error_without_acknowledging_success() {
    let server = Server::start().await;
    let (raw, _) = server
        .tokens
        .create_token("admin".into(), None, vec![Permission::Admin])
        .unwrap();
    let (mut stream, response) = server.connect(&raw).await;
    assert!(response.starts_with("HTTP/1.1 101"), "{response}");
    let sst_dir = server.directory.path().join("sstables");
    std::fs::remove_dir(&sst_dir).unwrap();
    std::fs::write(&sst_dir, b"blocks SSTable directory creation").unwrap();
    // The test engine has a 1 KiB memtable and flushes at 4 KiB.
    send(&mut stream, push("notes/failure", &"x".repeat(9000), 1)).await;
    assert_eq!(receive(&mut stream).await["type"], "error");
    assert!(server.sync.get_changes_since(0).is_empty());
    std::fs::remove_file(&sst_dir).unwrap();
    std::fs::create_dir(&sst_dir).unwrap();
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(5), server.handle.stop(true))
        .await
        .unwrap();
}

#[actix_web::test]
async fn broadcasts_and_pending_changes_respect_per_key_read_policies() {
    let mut controller = AccessController::new();
    for principal in ["alice", "bob"] {
        for operation in [Operation::Read, Operation::Write] {
            let name = format!("{principal}-{operation:?}");
            controller.set_policy(
                &name,
                AccessPolicy {
                    name: name.clone(),
                    operation,
                    key_pattern: format!("{principal}/*"),
                    effect: Effect::Allow,
                    context_matchers: [("principal".into(), principal.into())].into(),
                },
            );
        }
    }
    let server = Server::with_access(Some(controller)).await;
    let (alice_token, _) = server
        .tokens
        .create_token(
            "alice".into(),
            None,
            vec![Permission::Read, Permission::Write],
        )
        .unwrap();
    let (bob_token, _) = server
        .tokens
        .create_token(
            "bob".into(),
            None,
            vec![Permission::Read, Permission::Write],
        )
        .unwrap();
    let (mut alice, response) = server.connect(&alice_token).await;
    assert!(response.starts_with("HTTP/1.1 101"));
    let (mut bob, response) = server.connect(&bob_token).await;
    assert!(response.starts_with("HTTP/1.1 101"));
    send(&mut alice, push("alice/private", "alice-secret", 1)).await;
    let ack = receive(&mut alice).await;
    assert_eq!(ack["type"], "sync_ack");
    assert_eq!(ack["changes"][0]["key"], "alice/private");
    let broadcast = receive(&mut alice).await;
    assert_eq!(broadcast["type"], "sync_push");
    assert_eq!(broadcast["changes"][0]["key"], "alice/private");

    // The next frame Bob sees must be his own ack, never Alice's broadcast.
    send(&mut bob, push("bob/private", "bob-secret", 2)).await;
    let ack = receive(&mut bob).await;
    assert_eq!(ack["type"], "sync_ack");
    assert_eq!(ack["changes"].as_array().unwrap().len(), 1);
    assert_eq!(ack["changes"][0]["key"], "bob/private");
    let broadcast = receive(&mut bob).await;
    assert_eq!(broadcast["changes"][0]["key"], "bob/private");
    send(&mut bob, push("alice/forbidden", "injected", 3)).await;
    assert_eq!(receive(&mut bob).await["type"], "error");
    assert_eq!(server.engine.get("alice/forbidden").unwrap(), None);
    drop(alice);
    drop(bob);
    tokio::time::timeout(std::time::Duration::from_secs(5), server.handle.stop(true))
        .await
        .unwrap();
}

#[test]
fn stale_sync_changes_do_not_overwrite_the_persisted_winner() {
    use apexstore::api::sync::SyncChange;
    let (engine, _directory) = apexstore::api::test_helpers::test_engine();
    let manager = SyncManager::new();
    for (value, timestamp) in [("new", 2), ("old", 1), ("equal", 2)] {
        manager
            .apply_change(
                &SyncChange {
                    key: "notes/winner".into(),
                    value: value.into(),
                    timestamp,
                    device_id: "test".into(),
                },
                &engine,
                "default",
            )
            .unwrap();
    }
    assert_eq!(engine.get("notes/winner").unwrap(), Some(b"new".to_vec()));
    let changes = manager.get_changes_since(0);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].value, "new");
    assert_eq!(changes[0].timestamp, 2);
}
