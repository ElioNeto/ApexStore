//! TLS certificate loading shared by server startup and handshake tests.
use super::config::ServerConfig;

pub(super) fn load_tls_config(
    config: &ServerConfig,
) -> std::io::Result<Option<rustls::ServerConfig>> {
    let tls_config = if config.tls_enabled {
        let cert_path = config.tls_cert_path.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TLS enabled but TLS_CERT_PATH not set",
            )
        })?;
        let key_path = config.tls_key_path.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "TLS enabled but TLS_KEY_PATH not set",
            )
        })?;

        use std::io::BufReader;

        let cert_file = std::fs::File::open(cert_path).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Cannot open cert file: {}", e),
            )
        })?;
        let key_file = std::fs::File::open(key_path).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Cannot open key file: {}", e),
            )
        })?;

        let mut cert_reader = BufReader::new(cert_file);
        let mut key_reader = BufReader::new(key_file);

        let certs = rustls_pemfile::certs(&mut cert_reader)
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to parse cert: {}", e),
                )
            })?;

        let key = rustls_pemfile::private_key(&mut key_reader)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No private key found in key file",
            )
        })?;

        let tls_server_config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(std::io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("TLS config error: {}", e),
            )
        })?;

        Some(tls_server_config)
    } else {
        None
    };

    Ok(tls_config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn certificate_config(dir: &std::path::Path) -> (ServerConfig, Vec<u8>) {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert_path = dir.join("cert.pem");
        let key_path = dir.join("key.pem");
        std::fs::write(&cert_path, certified.cert.pem()).unwrap();
        std::fs::write(&key_path, certified.signing_key.serialize_pem()).unwrap();
        let config = ServerConfig {
            tls_enabled: true,
            tls_cert_path: Some(cert_path.to_string_lossy().into_owned()),
            tls_key_path: Some(key_path.to_string_lossy().into_owned()),
            ..ServerConfig::default()
        };
        (config, certified.cert.der().to_vec())
    }

    #[actix_web::test]
    async fn serves_https_with_a_verified_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let (config, cert_der) = certificate_config(dir.path());
        let tls = load_tls_config(&config).unwrap().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = actix_web::HttpServer::new(|| {
            actix_web::App::new().route("/", actix_web::web::get().to(|| async { "tls-ok" }))
        })
        .workers(1)
        .listen_rustls_0_23(listener, tls)
        .unwrap()
        .run();
        let handle = server.handle();
        let task = actix_web::rt::spawn(server);
        for version in [reqwest::Version::HTTP_11, reqwest::Version::HTTP_2] {
            let builder = reqwest::Client::builder()
                .add_root_certificate(reqwest::Certificate::from_der(&cert_der).unwrap())
                .no_proxy()
                .timeout(std::time::Duration::from_secs(10));
            let client = if version == reqwest::Version::HTTP_2 {
                builder.http2_prior_knowledge()
            } else {
                builder.http1_only()
            }
            .build()
            .unwrap();
            let response = client
                .get(format!("https://localhost:{port}/"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.version(), version);
            assert_eq!(
                response.error_for_status().unwrap().text().await.unwrap(),
                "tls-ok"
            );
        }
        handle.stop(true).await;
        task.await.unwrap().unwrap();
    }

    #[test]
    fn rejects_missing_and_invalid_tls_material() {
        let dir = tempfile::tempdir().unwrap();
        let (mut config, _) = certificate_config(dir.path());
        std::fs::write(config.tls_key_path.as_ref().unwrap(), "not a private key").unwrap();
        assert!(load_tls_config(&config).is_err());
        config.tls_cert_path = None;
        assert!(load_tls_config(&config).is_err());
    }
}
