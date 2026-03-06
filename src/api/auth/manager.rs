//! Token management and storage

use super::token::{generate_token, ApiToken, Permission};
use super::AuthError;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Token manager for storing and retrieving tokens
#[derive(Clone)]
pub struct TokenManager {
    tokens: Arc<RwLock<HashMap<String, ApiToken>>>,
}

impl TokenManager {
    /// Create new token manager
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new token
    pub fn create_token(
        &self,
        name: String,
        expires_at: Option<u128>,
        permissions: Vec<Permission>,
    ) -> Result<(String, ApiToken), AuthError> {
        let raw_token = generate_token();
        let token = ApiToken::new(name, &raw_token, expires_at, permissions);

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
                if token.is_expired() {
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
    pub fn delete_token(&self, id: &str) -> Result<(), AuthError> {
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
