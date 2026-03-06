//! Token structures and utilities

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

/// API token with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    /// Unique token identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// SHA-256 hash of the token
    pub token_hash: String,
    /// Creation timestamp (nanoseconds)
    pub created_at: u128,
    /// Optional expiry timestamp (nanoseconds)
    pub expires_at: Option<u128>,
    /// Granted permissions
    pub permissions: Vec<Permission>,
}

/// Permission levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    /// Read-only access
    Read,
    /// Write access (includes read)
    Write,
    /// Delete access (includes read)
    Delete,
    /// Administrative access (all permissions)
    Admin,
}

impl ApiToken {
    /// Create new token with given parameters
    pub fn new(
        name: String,
        raw_token: &str,
        expires_at: Option<u128>,
        permissions: Vec<Permission>,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let token_hash = hash_token(raw_token);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        Self {
            id,
            name,
            token_hash,
            created_at,
            expires_at,
            permissions,
        }
    }

    /// Check if token has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            now > expires_at
        } else {
            false
        }
    }

    /// Check if token has specific permission
    pub fn has_permission(&self, required: Permission) -> bool {
        if self.permissions.contains(&Permission::Admin) {
            return true;
        }
        self.permissions.contains(&required)
    }

    /// Validate raw token against stored hash
    pub fn validate_token(&self, raw_token: &str) -> bool {
        let hash = hash_token(raw_token);
        constant_time_compare(&hash, &self.token_hash)
    }
}

/// Generate a new random token
pub fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen::<u8>()).collect();

    format!("apx_{}", general_purpose::STANDARD.encode(&random_bytes))
}

/// Hash token using SHA-256
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Constant-time string comparison to prevent timing attacks
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation() {
        let token = generate_token();
        assert!(token.starts_with("apx_"));
        assert!(token.len() > 40);
    }

    #[test]
    fn test_token_hashing() {
        let token = "test_token_123";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_token_validation() {
        let raw_token = generate_token();
        let api_token = ApiToken::new("test".to_string(), &raw_token, None, vec![Permission::Read]);
        assert!(api_token.validate_token(&raw_token));
        assert!(!api_token.validate_token("wrong_token"));
    }

    #[test]
    fn test_token_expiry() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let expired = ApiToken::new(
            "test".to_string(),
            "token",
            Some(now - 1000),
            vec![Permission::Read],
        );
        assert!(expired.is_expired());

        let valid = ApiToken::new(
            "test".to_string(),
            "token",
            Some(now + 1_000_000_000),
            vec![Permission::Read],
        );
        assert!(!valid.is_expired());
    }

    #[test]
    fn test_permissions() {
        let token = ApiToken::new(
            "test".to_string(),
            "token",
            None,
            vec![Permission::Read, Permission::Write],
        );
        assert!(token.has_permission(Permission::Read));
        assert!(token.has_permission(Permission::Write));
        assert!(!token.has_permission(Permission::Delete));

        let admin = ApiToken::new("admin".to_string(), "token", None, vec![Permission::Admin]);
        assert!(admin.has_permission(Permission::Read));
        assert!(admin.has_permission(Permission::Write));
        assert!(admin.has_permission(Permission::Delete));
    }

    #[test]
    fn test_constant_time_compare() {
        assert!(constant_time_compare("hello", "hello"));
        assert!(!constant_time_compare("hello", "world"));
        assert!(!constant_time_compare("hello", "hello!"));
    }
}
