use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_json_payload_size: usize,
    pub max_raw_payload_size: usize,
    pub feature_cache_ttl_secs: u64,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// Enable/disable authentication
    pub enabled: bool,
    /// Token expiry in days (None = no expiry)
    pub token_expiry_days: Option<u32>,
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

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default for backward compatibility
            token_expiry_days: Some(30),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .unwrap_or(8080);

        let max_json_payload_size = env::var("MAX_JSON_PAYLOAD_SIZE")
            .unwrap_or_else(|_| (50 * 1024 * 1024).to_string())
            .parse::<usize>()
            .unwrap_or(50 * 1024 * 1024);

        let max_raw_payload_size = env::var("MAX_RAW_PAYLOAD_SIZE")
            .unwrap_or_else(|_| (50 * 1024 * 1024).to_string())
            .parse::<usize>()
            .unwrap_or(50 * 1024 * 1024);

        let feature_cache_ttl_secs = env::var("FEATURE_CACHE_TTL")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u64>()
            .unwrap_or(10);

        let auth_enabled = env::var("API_AUTH_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .unwrap_or(false);

        let token_expiry_days = env::var("API_TOKEN_EXPIRY_DAYS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok());

        Self {
            host,
            port,
            max_json_payload_size,
            max_raw_payload_size,
            feature_cache_ttl_secs,
            auth: AuthConfig {
                enabled: auth_enabled,
                token_expiry_days,
            },
        }
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
        println!();
    }
}
