//! Authentication error types

use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};
use serde_json::json;
use std::fmt;

/// Authentication result type
pub type AuthResult<T> = Result<T, AuthError>;

/// Authentication error types
#[derive(Debug, Clone)]
pub enum AuthError {
    /// Invalid or malformed token
    InvalidToken,
    /// Token has expired
    TokenExpired,
    /// Missing authorization header
    MissingToken,
    /// Insufficient permissions
    InsufficientPermissions,
    /// Token not found in store
    TokenNotFound,
    /// Token generation failed
    TokenGenerationFailed,
    /// Internal error
    Internal(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::InvalidToken => write!(f, "Invalid authentication token"),
            AuthError::TokenExpired => write!(f, "Token has expired"),
            AuthError::MissingToken => write!(f, "Missing authorization header"),
            AuthError::InsufficientPermissions => write!(f, "Insufficient permissions"),
            AuthError::TokenNotFound => write!(f, "Token not found"),
            AuthError::TokenGenerationFailed => write!(f, "Failed to generate token"),
            AuthError::Internal(msg) => write!(f, "Internal auth error: {}", msg),
        }
    }
}

impl std::error::Error for AuthError {}

impl ResponseError for AuthError {
    fn status_code(&self) -> StatusCode {
        match self {
            AuthError::InvalidToken | AuthError::TokenExpired | AuthError::MissingToken => {
                StatusCode::UNAUTHORIZED
            }
            AuthError::InsufficientPermissions => StatusCode::FORBIDDEN,
            AuthError::TokenNotFound => StatusCode::NOT_FOUND,
            AuthError::TokenGenerationFailed | AuthError::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).json(json!({
            "error": self.to_string(),
            "status": self.status_code().as_u16(),
        }))
    }
}
