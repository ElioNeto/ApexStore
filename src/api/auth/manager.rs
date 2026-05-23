//! Token management and storage

use super::token::{generate_token, ApiToken, Permission};
use super::AuthError;
use crate::LsmEngine;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Prefix used for storing API tokens in the engine
const TOKEN_PREFIX: &str = "__token:";

/// Token manager for storing and retrieving tokens
///
/// Tokens are cached in a memory HashMap for fast access and optionally
/// persisted in the LSM engine under the `__token:*` prefix for durability
/// across server restarts.
#[derive(Clone)]
pub struct TokenManager {
    tokens: Arc<RwLock<HashMap<String, ApiToken>>>,
    engine: Option<Arc<LsmEngine>>,
}

impl TokenManager {
    /// Create new token manager (in-memory only, no persistence)
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            engine: None,
        }
    }

    /// Create new token manager with engine persistence.
    ///
    /// All existing tokens stored under the `__token:*` prefix are loaded
    /// into memory on construction. Subsequent `create_token` and
    /// `delete_token` calls are automatically persisted to the engine.
    pub fn new_with_engine(engine: Arc<LsmEngine>) -> Self {
        let manager = Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            engine: Some(engine),
        };
        if let Err(e) = manager.load_tokens_from_engine() {
            tracing::warn!(target: "apexstore::auth", "Failed to load tokens from engine: {}", e);
        }
        manager
    }

    /// Load all `__token:*` entries from the engine into the in-memory cache.
    fn load_tokens_from_engine(&self) -> Result<(), AuthError> {
        if let Some(ref engine) = self.engine {
            use crate::core::engine::MAX_SCAN_LIMIT;
            let (results, _cursor) = engine
                .search_prefix(TOKEN_PREFIX, None, MAX_SCAN_LIMIT)
                .map_err(|e| AuthError::Internal(format!("Engine scan error: {}", e)))?;

            let mut tokens = self
                .tokens
                .write()
                .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

            for (_key, value) in &results {
                match serde_json::from_slice::<ApiToken>(value) {
                    Ok(token) => {
                        tokens.insert(token.id.clone(), token);
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "apexstore::auth",
                            "Failed to deserialize token from engine: {}",
                            e
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Persist a single token to the engine (if engine is configured).
    fn persist_token(&self, token: &ApiToken) -> Result<(), AuthError> {
        if let Some(ref engine) = self.engine {
            let key = format!("{}{}", TOKEN_PREFIX, token.id);
            let value = serde_json::to_vec(token)
                .map_err(|e| AuthError::Internal(format!("Serialization error: {}", e)))?;
            engine
                .put_cf("default", key.as_bytes().to_vec(), value)
                .map_err(|e| AuthError::Internal(format!("Engine write error: {}", e)))?;
        }
        Ok(())
    }

    /// Remove a single token from the engine (if engine is configured).
    fn delete_persisted_token(&self, id: &str) -> Result<(), AuthError> {
        if let Some(ref engine) = self.engine {
            let key = format!("{}{}", TOKEN_PREFIX, id);
            engine
                .delete_cf("default", key.as_bytes())
                .map_err(|e| AuthError::Internal(format!("Engine delete error: {}", e)))?;
        }
        Ok(())
    }

    /// Create a new token.
    ///
    /// The token is persisted to the engine before being added to the
    /// in-memory cache. If persistence fails the create is aborted.
    pub fn create_token(
        &self,
        name: String,
        expires_at: Option<u128>,
        permissions: Vec<Permission>,
    ) -> Result<(String, ApiToken), AuthError> {
        let raw_token = generate_token();
        let token = ApiToken::new(name, &raw_token, expires_at, permissions)?;

        // Persist to engine first (crash-safe: on restart the token is reloaded)
        self.persist_token(&token)?;

        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        tokens.insert(token.id.clone(), token.clone());

        Ok((raw_token, token))
    }

    /// Validate a token and return the ApiToken if valid
    pub fn validate_token(&self, raw_token: &str) -> Result<ApiToken, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        for token in tokens.values() {
            if token.validate_token(raw_token) {
                if token.is_expired()? {
                    return Err(AuthError::TokenExpired);
                }
                return Ok(token.clone());
            }
        }

        Err(AuthError::InvalidToken)
    }

    /// List all tokens (without raw token values)
    pub fn list_tokens(&self) -> Result<Vec<ApiToken>, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        Ok(tokens.values().cloned().collect())
    }

    /// Get token by ID
    pub fn get_token(&self, id: &str) -> Result<ApiToken, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        tokens.get(id).cloned().ok_or(AuthError::TokenNotFound)
    }

    /// Delete token by ID
    ///
    /// The token is removed from the engine first, then from the in-memory
    /// cache. If the engine delete fails the operation is aborted to keep
    /// persistence consistent.
    pub fn delete_token(&self, id: &str) -> Result<(), AuthError> {
        // Delete from engine first (crash-safe: on restart the token is
        // still gone from the engine, stale cache is discarded on next load)
        self.delete_persisted_token(id)?;

        let mut tokens = self
            .tokens
            .write()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        tokens.remove(id).ok_or(AuthError::TokenNotFound)?;
        Ok(())
    }

    /// Get count of active tokens
    pub fn count(&self) -> Result<usize, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        Ok(tokens.len())
    }
}

impl Default for TokenManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_validate_token() {
        let manager = TokenManager::new();
        let (raw_token, token) = manager
            .create_token("test".to_string(), None, vec![Permission::Read])
            .unwrap();

        let validated = manager.validate_token(&raw_token).unwrap();
        assert_eq!(validated.id, token.id);
        assert_eq!(validated.name, "test");
    }

    #[test]
    fn test_invalid_token() {
        let manager = TokenManager::new();
        let result = manager.validate_token("invalid_token");
        assert!(matches!(result, Err(AuthError::InvalidToken)));
    }

    #[test]
    fn test_list_tokens() {
        let manager = TokenManager::new();
        manager
            .create_token("token1".to_string(), None, vec![Permission::Read])
            .unwrap();
        manager
            .create_token("token2".to_string(), None, vec![Permission::Write])
            .unwrap();

        let tokens = manager.list_tokens().unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_delete_token() {
        let manager = TokenManager::new();
        let (_, token) = manager
            .create_token("test".to_string(), None, vec![Permission::Read])
            .unwrap();

        assert_eq!(manager.count().unwrap(), 1);
        manager.delete_token(&token.id).unwrap();
        assert_eq!(manager.count().unwrap(), 0);
    }

    #[test]
    fn test_expired_token() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let manager = TokenManager::new();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let (raw_token, _) = manager
            .create_token(
                "expired".to_string(),
                Some(now - 1000),
                vec![Permission::Read],
            )
            .unwrap();

        let result = manager.validate_token(&raw_token);
        assert!(matches!(result, Err(AuthError::TokenExpired)));
    }
}
