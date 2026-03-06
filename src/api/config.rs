use super::auth::AuthConfig;
use std::env;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_json_payload_size: usize,
    pub max_raw_payload_size: usize,
    pub feature_cache_ttl_secs: u64,
    pub auth: AuthConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            max_json_payload_size: 50 * 1024 * 1024, // 50MB
            max_raw_payload_size: 50 * 1024 * 1024,  // 50MB
            feature_cache_ttl_secs: 10,
            auth: AuthConfig::default(),
        }
    }
}

impl ServerConfig {
    pub fn new(
        host: String,
        port: u16,
        max_json_payload_size: usize,
        max_raw_payload_size: usize,
        feature_cache_ttl_secs: u64,
        auth: AuthConfig,
    ) -> Self {
        Self {
            host,
            port,
            max_json_payload_size,
            max_raw_payload_size,
            feature_cache_ttl_secs,
            auth,
        }
    }

    pub fn from_env() -> Self {
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .expect("PORT must be a valid u16");

        let max_json_payload_size = env::var("MAX_JSON_PAYLOAD_SIZE")
            .unwrap_or_else(|_| "52428800".to_string())
            .parse::<usize>()
            .expect("MAX_JSON_PAYLOAD_SIZE must be a valid usize");

        let max_raw_payload_size = env::var("MAX_RAW_PAYLOAD_SIZE")
            .unwrap_or_else(|_| "52428800".to_string())
            .parse::<usize>()
            .expect("MAX_RAW_PAYLOAD_SIZE must be a valid usize");

        let feature_cache_ttl_secs = env::var("FEATURE_CACHE_TTL")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u64>()
            .expect("FEATURE_CACHE_TTL must be a valid u64");

        let auth = AuthConfig::from_env();

        Self {
            host,
            port,
            max_json_payload_size,
            max_raw_payload_size,
            feature_cache_ttl_secs,
            auth,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("Port cannot be 0".to_string());
        }

        if self.max_json_payload_size == 0 || self.max_raw_payload_size == 0 {
            return Err("Payload sizes must be > 0".to_string());
        }

        Ok(())
    }

    pub fn print_info(&self) {
        println!("📋 Server Configuration:");
        println!("   Host: {}", self.host);
        println!("   Port: {}", self.port);
        println!(
            "   JSON Payload Limit: {} MB",
            self.max_json_payload_size / 1024 / 1024
        );
        println!(
            "   Raw Payload Limit: {} MB",
            self.max_raw_payload_size / 1024 / 1024
        );
        println!("   Feature Cache TTL: {}s", self.feature_cache_ttl_secs);
        println!(
            "   Authentication: {}",
            if self.auth.enabled {
                "Enabled"
            } else {
                "Disabled"
            }
        );
        if let Some(days) = self.auth.token_expiry_days {
            println!("   Token Expiry: {} days", days);
        } else {
            println!("   Token Expiry: Never");
        }
    }
}
