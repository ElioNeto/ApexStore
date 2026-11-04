use apexstore::api::auth::{ApiToken, Permission, TokenManager};
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::{LsmConfig, LsmEngine};
use std::sync::Arc;

fn open(path: &std::path::Path) -> Arc<LsmEngine> {
    let config = LsmConfig::builder()
        .dir_path(path.to_path_buf())
        .build()
        .unwrap();
    Arc::new(LsmEngine::new_from_config(&config, GlobalBlockCache::new(1, 4096)).unwrap())
}

#[test]
fn forged_user_record_is_not_admitted_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let forged = ApiToken::new(
        "forged".into(),
        "attacker-chosen-token",
        None,
        vec![Permission::Admin],
    )
    .unwrap();
    {
        let engine = open(directory.path());
        engine
            .set(
                format!("__token:{}", forged.id),
                serde_json::to_vec(&forged).unwrap(),
            )
            .unwrap();
        engine.close();
    }
    let engine = open(directory.path());
    let manager = TokenManager::new_with_engine(engine).unwrap();
    assert!(manager.validate_token("attacker-chosen-token").is_err());
    assert_eq!(manager.count().unwrap(), 0);
}

#[test]
fn trusted_tokens_are_not_records_in_the_public_keyspace() {
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (raw, token) = manager
        .create_token("trusted".into(), None, vec![Permission::Admin])
        .unwrap();
    assert!(manager.validate_token(&raw).is_ok());
    assert_eq!(engine.get(format!("__token:{}", token.id)).unwrap(), None);
    assert!(engine.scan().unwrap().is_empty());
}

#[test]
fn private_tokens_survive_restart_and_revocation_is_durable() {
    let directory = tempfile::tempdir().unwrap();
    let (raw, id) = {
        let engine = open(directory.path());
        let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
        let (raw, token) = manager
            .create_token("persistent".into(), None, vec![Permission::Admin])
            .unwrap();
        engine.set("public", b"data").unwrap();
        engine.flush_memtable().unwrap();
        (raw, token.id)
    };
    {
        let engine = open(directory.path());
        let manager = TokenManager::new_with_engine(engine).unwrap();
        assert!(manager.validate_token(&raw).is_ok());
        manager.delete_token(&id).unwrap();
        assert!(manager.validate_token(&raw).is_err());
    }
    let manager = TokenManager::new_with_engine(open(directory.path())).unwrap();
    assert!(manager.validate_token(&raw).is_err());
    assert_eq!(manager.count().unwrap(), 0);
}

#[test]
fn corrupted_credentials_fail_closed_without_replacing_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    {
        let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
        manager
            .create_token("trusted".into(), None, vec![Permission::Admin])
            .unwrap();
    }
    let file = directory.path().join(".apex-auth/tokens.json");
    std::fs::write(&file, b"{truncated credentials").unwrap();
    assert!(TokenManager::new_with_engine(engine).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"{truncated credentials");
}

#[test]
fn failed_persistence_does_not_change_live_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (raw, token) = manager
        .create_token("trusted".into(), None, vec![Permission::Admin])
        .unwrap();
    let auth = directory.path().join(".apex-auth");
    let moved = directory.path().join("saved-auth");
    std::fs::rename(&auth, &moved).unwrap();
    std::fs::write(&auth, b"not a directory").unwrap();
    assert!(manager
        .create_token("unpersisted".into(), None, vec![Permission::Admin])
        .is_err());
    assert!(manager.delete_token(&token.id).is_err());
    assert_eq!(manager.count().unwrap(), 1);
    assert!(manager.validate_token(&raw).is_ok());
    std::fs::remove_file(&auth).unwrap();
    std::fs::rename(&moved, &auth).unwrap();
    drop(manager);
    let reopened = TokenManager::new_with_engine(engine).unwrap();
    assert_eq!(reopened.count().unwrap(), 1);
    assert!(reopened.validate_token(&raw).is_ok());
}

#[test]
fn concurrent_store_owners_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    let first = TokenManager::new_with_engine(engine.clone()).unwrap();
    assert!(TokenManager::new_with_engine(engine.clone()).is_err());
    drop(first);
    assert!(TokenManager::new_with_engine(engine).is_ok());
}

#[test]
fn cli_uses_the_same_private_store_as_the_server() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_apexstore-cli"))
        .arg("--db")
        .arg(directory.path())
        .args(["token", "create", "offline-admin", "--permissions", "admin"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let raw = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Token: "))
        .unwrap();
    let engine = open(directory.path());
    let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
    assert!(manager
        .validate_token(raw)
        .unwrap()
        .has_permission(Permission::Admin));
    assert!(engine.scan().unwrap().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::metadata(directory.path().join(".apex-auth/tokens.json"))
            .unwrap()
            .permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);
    }
}

#[actix_web::test]
async fn public_api_rejects_reserved_keys_for_all_operations() {
    use actix_web::{http::StatusCode, test as http_test, web, App};
    use actix_web_httpauth::middleware::HttpAuthentication;
    use apexstore::api::{auth::request_validator, config::AuthEnabled, configure};
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    let manager = TokenManager::new_with_engine(engine.clone()).unwrap();
    let (raw, _) = manager
        .create_token("admin".into(), None, vec![Permission::Admin])
        .unwrap();
    let app = http_test::init_service(
        App::new()
            .app_data(web::Data::from(engine.clone()))
            .app_data(web::Data::new(manager.clone()))
            .app_data(web::Data::new(AuthEnabled(true)))
            .wrap(HttpAuthentication::with_fn(request_validator))
            .configure(configure),
    )
    .await;
    for prefix in ["__token:", "__ttl:", "__index:", "__idx:"] {
        let key = format!("{prefix}fixture");
        engine.set(key.as_bytes(), b"internal-value").unwrap();
        for request in [
            http_test::TestRequest::get().uri(&format!("/keys/{key}")),
            http_test::TestRequest::put()
                .uri(&format!("/keys/{key}"))
                .set_json(serde_json::json!({"value":"changed"})),
            http_test::TestRequest::post()
                .uri("/keys")
                .set_json(serde_json::json!({"key":key,"value":"changed"})),
            http_test::TestRequest::delete().uri(&format!("/keys/{key}")),
            http_test::TestRequest::get().uri(&format!("/keys?prefix={prefix}")),
            http_test::TestRequest::delete().uri(&format!("/keys?prefix={prefix}")),
        ] {
            let response = http_test::call_service(
                &app,
                request
                    .insert_header(("Authorization", format!("Bearer {raw}")))
                    .to_request(),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "reserved prefix {prefix}"
            );
        }
        assert_eq!(engine.get(&key).unwrap(), Some(b"internal-value".to_vec()));
    }
    let response = http_test::call_service(
        &app,
        http_test::TestRequest::delete()
            .uri("/keys?prefix=__")
            .insert_header(("Authorization", format!("Bearer {raw}")))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(manager.validate_token(&raw).is_ok());
}

#[actix_web::test]
async fn public_scans_hide_legacy_metadata_without_consuming_the_limit() {
    use actix_web::{test as http_test, web, App};
    use apexstore::api::{config::AuthEnabled, configure};
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    engine.set("__token:legacy", b"private-metadata").unwrap();
    engine.set("__idx:legacy", b"private-metadata").unwrap();
    for index in 0..300 {
        engine
            .set(format!("__token:legacy-{index:04}"), b"private-metadata")
            .unwrap();
    }
    engine.set("z-visible", b"public-data").unwrap();
    let app = http_test::init_service(
        App::new()
            .app_data(web::Data::from(engine))
            .app_data(web::Data::new(AuthEnabled(false)))
            .configure(configure),
    )
    .await;
    for uri in [
        "/scan?limit=1",
        "/keys?limit=1",
        "/keys?prefix=&limit=1",
        "/keys/search?prefix=&limit=1",
    ] {
        let response =
            http_test::call_service(&app, http_test::TestRequest::get().uri(uri).to_request())
                .await;
        assert!(response.status().is_success());
        let body: serde_json::Value = http_test::read_body_json(response).await;
        let serialized = body.to_string();
        assert!(serialized.contains("z-visible"), "{uri}: {body}");
        assert!(!serialized.contains("__token:"), "{uri}: {body}");
        assert!(!serialized.contains("__idx:"), "{uri}: {body}");
        assert!(!serialized.contains("private-metadata"), "{uri}: {body}");
    }
}

#[actix_web::test]
async fn graphql_cannot_read_or_modify_reserved_metadata() {
    use apexstore::api::{config::AuthEnabled, graphql::build_schema};
    let directory = tempfile::tempdir().unwrap();
    let engine = open(directory.path());
    engine.set("__token:legacy", b"private-metadata").unwrap();
    engine.set("visible", b"public").unwrap();
    let schema = build_schema(engine.clone());
    let response = schema
        .execute(
            async_graphql::Request::new(
                r#"{ get(key: "__token:legacy") scan(limit: 1) { key value } }"#,
            )
            .data(AuthEnabled(false)),
        )
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap(),
        serde_json::json!({
            "get":null,"scan":[{"key":"visible","value":"public"}]
        })
    );
    let keys = schema
        .execute(async_graphql::Request::new("{ keys }").data(AuthEnabled(false)))
        .await;
    assert!(keys.errors.is_empty());
    assert_eq!(
        keys.data.into_json().unwrap(),
        serde_json::json!({"keys":["visible"]})
    );
    let response = schema.execute(async_graphql::Request::new(
        r#"mutation { set(key: "__token:forged", value: "forged") delete(key: "__token:legacy") }"#,
    ).data(AuthEnabled(false))).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap(),
        serde_json::json!({"set":false,"delete":false})
    );
    assert_eq!(engine.get("__token:forged").unwrap(), None);
    assert_eq!(
        engine.get("__token:legacy").unwrap(),
        Some(b"private-metadata".to_vec())
    );
}

#[actix_web::test]
async fn server_process_refuses_corrupted_credential_storage() {
    let directory = tempfile::tempdir().unwrap();
    {
        let engine = open(directory.path());
        let manager = TokenManager::new_with_engine(engine).unwrap();
        manager
            .create_token("trusted".into(), None, vec![Permission::Admin])
            .unwrap();
    }
    let file = directory.path().join(".apex-auth/tokens.json");
    std::fs::write(&file, b"{corrupt snapshot").unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_apexstore-server"))
            .env("DATA_DIR", directory.path())
            .env("API_AUTH_ENABLED", "true")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("server must fail startup promptly")
    .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Private token storage"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(file).unwrap(), b"{corrupt snapshot");
}
