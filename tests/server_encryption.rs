use apexstore::{infra::config::LsmConfig, storage::cache::GlobalBlockCache, LsmEngine};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
fn server(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apexstore-server"));
    command
        .current_dir(dir)
        .env("DATA_DIR", dir.join("database"))
        .env("HOST", "127.0.0.1")
        .env("API_AUTH_ENABLED", "false")
        .env("ACCESS_CONTROL_ENABLED", "false")
        .env("WORKERS", "1")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("ENCRYPTION_KEY_PATH");
    command
}
#[test]
fn server_rejects_invalid_encryption_environment_before_opening_storage() {
    for enabled in ["yes", "true"] {
        let dir = tempfile::tempdir().unwrap();
        let result = server(dir.path())
            .env("ENCRYPTION_ENABLED", enabled)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!dir.path().join("database").exists());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains(if enabled == "yes" {
                "ENCRYPTION_ENABLED"
            } else {
                "encryption key"
            }),
            "{error}"
        );
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
async fn server_environment_controls_real_http_writes_and_restart() {
    for encrypted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        std::fs::write(&key, hex::encode([0x41; 32])).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut running = RunningServer(
            server(dir.path())
                .env("PORT", port.to_string())
                .env("ENCRYPTION_ENABLED", encrypted.to_string())
                .env("ENCRYPTION_KEY_PATH", &key)
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
        let base = format!("http://127.0.0.1:{port}");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if client
                .get(format!("{base}/health/liveness"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                break;
            }
            assert!(
                running.0.try_wait().unwrap().is_none(),
                "server exited before listening"
            );
            assert!(std::time::Instant::now() < deadline, "server did not start");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let response = client
            .put(format!("{base}/keys/server-key"))
            .json(&serde_json::json!({"value":"persisted"}))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
        let response = client
            .post(format!("{base}/admin/flush"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "{}",
            response.text().await.unwrap()
        );
        drop(running);
        let paths: Vec<_> = std::fs::read_dir(dir.path().join("database/sstables"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "sst"))
            .collect();
        assert!(!paths.is_empty());
        for path in paths {
            assert_eq!(
                &std::fs::read(path).unwrap()[..8],
                if encrypted { b"LSMSST04" } else { b"LSMSST03" }
            );
        }
        let mut cfg = LsmConfig::default();
        cfg.core.dir_path = dir.path().join("database");
        cfg.storage.encryption_enabled = encrypted;
        cfg.storage.encryption_key_path = Some(key.to_str().unwrap().to_owned());
        let engine = LsmEngine::new_from_config(&cfg, GlobalBlockCache::new(1, 4096)).unwrap();
        assert_eq!(
            engine.get("server-key").unwrap(),
            Some(b"persisted".to_vec())
        );
    }
}
