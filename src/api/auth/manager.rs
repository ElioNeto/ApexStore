use super::token::Token;
use crate::infra::error::LsmError;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid token")]
    InvalidToken,

    #[error("Token not found")]
    TokenNotFound,

    #[error("Token expired")]
    TokenExpired,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<AuthError> for LsmError {
    fn from(err: AuthError) -> Self {
        LsmError::AuthenticationFailed(err.to_string())
    }
}

/// Token manager for handling authentication tokens
#[derive(Clone)]
pub struct TokenManager {
    tokens: Arc<RwLock<HashMap<String, Token>>>,
    expiry_days: Option<u32>,
}

impl TokenManager {
    pub fn new(expiry_days: Option<u32>) -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            expiry_days,
        }
    }

    /// Create a new token
    pub fn create_token(&self, name: String) -> Result<Token, AuthError> {
        let token = Token::new(name, self.expiry_days);

        self.tokens
            .write()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?
            .insert(token.id.clone(), token.clone());

        Ok(token)
    }

    /// Validate a token by its value
    pub fn validate_token(&self, token_value: &str) -> Result<Token, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        tokens
            .values()
            .find(|t| t.value == token_value)
            .cloned()
            .ok_or(AuthError::InvalidToken)
            .and_then(|token| {
                if token.is_expired() {
                    Err(AuthError::TokenExpired)
                } else {
                    Ok(token)
                }
            })
    }

    /// Get token by ID
    pub fn get_token(&self, id: &str) -> Result<Token, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        tokens.get(id).cloned().ok_or(AuthError::TokenNotFound)
    }

    /// Delete token by ID
    pub fn delete_token(&self, id: &str) -> Result<(), AuthError> {
        self.tokens
            .write()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?
            .remove(id)
            .ok_or(AuthError::TokenNotFound)?;

        Ok(())
    }

    /// List all tokens
    pub fn list_tokens(&self) -> Result<Vec<Token>, AuthError> {
        let tokens = self
            .tokens
            .read()
            .map_err(|e| AuthError::Internal(format!("Lock poisoned: {}", e)))?;

        Ok(tokens.values().cloned().collect())
    }
}
