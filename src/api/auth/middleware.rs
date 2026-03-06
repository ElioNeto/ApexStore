//! Authentication middleware for Actix-Web

use super::error::AuthError;
use super::manager::TokenManager;
use super::token::ApiToken;
use actix_web::dev::ServiceRequest;
use actix_web::Error;
use actix_web::HttpMessage;

/// Bearer token validator for HTTP authentication middleware
pub async fn bearer_validator(
    req: ServiceRequest,
    token_manager: TokenManager,
    credentials: Option<String>,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    let token = match credentials {
        Some(t) => t,
        None => return Err((AuthError::MissingToken.into(), req)),
    };

    match token_manager.validate_token(&token) {
        Ok(api_token) => {
            // Store token in request extensions for use in handlers
            req.extensions_mut().insert(api_token);
            Ok(req)
        }
        Err(e) => Err((e.into(), req)),
    }
}

/// Extract token from request extensions
pub fn extract_token(req: &actix_web::HttpRequest) -> Option<ApiToken> {
    req.extensions().get::<ApiToken>().cloned()
}
