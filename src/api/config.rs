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

    /// Maximum number of concurrent connections (default: 10000)
    pub max_connections: usize,
    /// TCP listen backlog size (default: 1024)
    pub backlog: u32,
    /// Number of worker threads (None = auto-detect based on CPU cores)
    pub workers: Option<usize>,
    /// Enable/disable IP-based rate limiting (default: true)
    pub rate_limit_enabled: bool,
    /// Max requests per minute per IP (default: 100)
    pub rate_limit_requests_per_minute: usize,

    /// CDC endpoint URL for streaming data changes.
    /// When set, CDC is enabled and data mutations are posted as JSON to this endpoint.
    pub cdc_endpoint: Option<String>,

    /// Enable/disable CORS middleware (default: true)
    pub cors_enabled: bool,
    /// Comma-separated allowed origins for CORS (empty = allow all)
    pub cors_origins: Option<Vec<String>>,
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
            max_connections: 10_000,
            backlog: 1024u32,
            workers: None,
            rate_limit_enabled: true,
            rate_limit_requests_per_minute: 100,
            cdc_endpoint: None,
            cors_enabled: true,
            cors_origins: None,
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

        let max_connections = env::var("MAX_CONNECTIONS")
            .unwrap_or_else(|_| "10000".to_string())
            .parse::<usize>()
            .unwrap_or(10_000);

        let backlog = env::var("BACKLOG")
            .unwrap_or_else(|_| "1024".to_string())
            .parse::<u32>()
            .unwrap_or(1024);

        let workers = env::var("WORKERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok());

        let rate_limit_enabled = env::var("RATE_LIMIT_ENABLED")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        let rate_limit_requests_per_minute = env::var("RATE_LIMIT_REQUESTS_PER_MINUTE")
            .unwrap_or_else(|_| "100".to_string())
            .parse::<usize>()
            .unwrap_or(100);

        let cdc_endpoint = env::var("CDC_ENDPOINT").ok();

        let cors_enabled = env::var("CORS_ENABLED")
            .unwrap_or_else(|_| "true".to_string())
            .parse::<bool>()
            .unwrap_or(true);

        let cors_origins_str = env::var("CORS_ORIGINS").ok();
        let cors_origins = cors_origins_str
            .filter(|s| !s.is_empty())
            .map(|s| s.split(',').map(|o| o.trim().to_string()).collect());

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
            max_connections,
            backlog,
            workers,
            rate_limit_enabled,
            rate_limit_requests_per_minute,
            cdc_endpoint,
            cors_enabled,
            cors_origins,
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
        println!("   Max Connections: {}", self.max_connections);
        println!("   Backlog: {}", self.backlog);
        match self.workers {
            Some(w) => println!("   Workers: {}", w),
            None => println!("   Workers: auto (CPU cores)"),
        }
        println!(
            "   Rate Limiting: {}",
            if self.rate_limit_enabled {
                format!(
                    "Enabled ({} req/min/IP)",
                    self.rate_limit_requests_per_minute
                )
            } else {
                "Disabled".to_string()
            }
        );
        println!(
            "   CDC: {}",
            match &self.cdc_endpoint {
                Some(url) => format!("Enabled ({})", url),
                None => "Disabled".to_string(),
            }
        );
        println!(
            "   CORS: {}",
            if self.cors_enabled {
                match &self.cors_origins {
                    Some(origins) => format!("Enabled (origins: {})", origins.join(", ")),
                    None => "Enabled (all origins allowed)".to_string(),
                }
            } else {
                "Disabled".to_string()
            }
        );
        println!();
    }
}
