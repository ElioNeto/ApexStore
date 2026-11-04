//! Token structures and utilities

use super::AuthError;
use base64::{engine::general_purpose, Engine as _};
use rand::{rngs::SysRng, TryCryptoRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;
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
    /// Write access (grant Read separately)
    Write,
    /// Delete access (grant Read separately)
    Delete,
    /// Administrative access (all permissions)
    Admin,
}

impl ApiToken {
    /// Create new token with given parameters.
    ///
    /// Returns an `AuthError::Internal` if the system clock is not
    /// functioning correctly (e.g. container without a clock, kernel bug).
    pub fn new(
        name: String,
        raw_token: &str,
        expires_at: Option<u128>,
        permissions: Vec<Permission>,
    ) -> Result<Self, AuthError> {
        let id = uuid::Uuid::new_v4().to_string();
        let token_hash = hash_token(raw_token);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AuthError::Internal(format!("SystemTime error: {}", e)))?
            .as_nanos();

        Ok(Self {
            id,
            name,
            token_hash,
            created_at,
            expires_at,
            permissions,
        })
    }

    /// Check if token has expired.
    ///
    /// Returns an `AuthError::Internal` if the system clock is not
    /// functioning correctly.
    pub fn is_expired(&self) -> Result<bool, AuthError> {
        if let Some(expires_at) = self.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| AuthError::Internal(format!("SystemTime error: {}", e)))?
                .as_nanos();
            Ok(now > expires_at)
        } else {
            Ok(false)
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

/// Generate a new token from 32 bytes of operating-system randomness.
///
/// Returns an error if the operating system cannot supply secure randomness.
pub fn generate_token() -> Result<String, AuthError> {
    generate_token_with_rng(&mut SysRng)
}

fn generate_token_with_rng(rng: &mut impl TryCryptoRng) -> Result<String, AuthError> {
    let mut random_bytes = [0u8; 32];
    rng.try_fill_bytes(&mut random_bytes)
        .map_err(|e| AuthError::Internal(format!("Secure token randomness unavailable: {e}")))?;

    Ok(format!(
        "apx_{}",
        general_purpose::STANDARD.encode(random_bytes)
    ))
}

impl FromStr for Permission {
    type Err = AuthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "read" | "r" => Ok(Permission::Read),
            "write" | "w" => Ok(Permission::Write),
            "delete" | "d" => Ok(Permission::Delete),
            "admin" | "a" => Ok(Permission::Admin),
            _ => Err(AuthError::InvalidPermission(s.to_string())),
        }
    }
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

    struct FailingRng;

    impl rand::TryRng for FailingRng {
        type Error = std::io::Error;

        fn try_next_u32(&mut self) -> std::io::Result<u32> {
            Err(std::io::Error::other("entropy unavailable"))
        }

        fn try_next_u64(&mut self) -> std::io::Result<u64> {
            Err(std::io::Error::other("entropy unavailable"))
        }

        fn try_fill_bytes(&mut self, dest: &mut [u8]) -> std::io::Result<()> {
            // A failed source may have partially written the destination.
            dest[0] = 0xAB;
            Err(std::io::Error::other("entropy unavailable"))
        }
    }

    impl rand::TryCryptoRng for FailingRng {}

    #[test]
    fn test_token_generation_rejects_entropy_failure() {
        let error = generate_token_with_rng(&mut FailingRng).unwrap_err();
        assert!(
            matches!(error, AuthError::Internal(message) if message.contains("entropy unavailable"))
        );
    }

    #[test]
    fn test_token_generation() {
        let token = generate_token().unwrap();
        assert!(token.starts_with("apx_"));
        assert_eq!(token.len(), 48);
        assert_eq!(
            general_purpose::STANDARD.decode(&token[4..]).unwrap().len(),
            32
        );
        assert_ne!(token, generate_token().unwrap());
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
        let raw_token = generate_token().unwrap();
        let api_token =
            ApiToken::new("test".to_string(), &raw_token, None, vec![Permission::Read]).unwrap();
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
        )
        .unwrap();
        assert!(expired.is_expired().unwrap());

        let valid = ApiToken::new(
            "test".to_string(),
            "token",
            Some(now + 1_000_000_000),
            vec![Permission::Read],
        )
        .unwrap();
        assert!(!valid.is_expired().unwrap());
    }

    #[test]
    fn test_permissions() {
        let token = ApiToken::new(
            "test".to_string(),
            "token",
            None,
            vec![Permission::Read, Permission::Write],
        )
        .unwrap();
        assert!(token.has_permission(Permission::Read));
        assert!(token.has_permission(Permission::Write));
        assert!(!token.has_permission(Permission::Delete));

        let admin =
            ApiToken::new("admin".to_string(), "token", None, vec![Permission::Admin]).unwrap();
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
