//! Authentication middleware for Actix-Web

use super::error::AuthError;
use super::manager::TokenManager;
use super::token::ApiToken;
use actix_web::dev::ServiceRequest;
use actix_web::web;
use actix_web::Error;
use actix_web::HttpMessage;
use actix_web_httpauth::extractors::bearer::BearerAuth;

/// Bearer token validator for HTTP authentication middleware.
///
/// Compatible with `actix-web-httpauth::HttpAuthentication::bearer`.
/// Checks whether authentication is enabled (via `AuthConfig` stored in
/// app data) and, if so, validates the bearer token using the `TokenManager`
/// also stored in app data.
///
/// When authentication is disabled all requests are allowed through.
pub async fn bearer_validator(
    req: ServiceRequest,
    credentials: BearerAuth,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    // Check if auth is enabled via the flag stored in app_data by start_server
    let auth_enabled = req
        .app_data::<web::Data<bool>>()
        .map(|flag| *flag.as_ref())
        .unwrap_or(false);

    if !auth_enabled {
        return Ok(req);
    }

    let token = credentials.token().to_string();

    // Extract TokenManager from app_data (injected by start_server)
    let token_manager = match req.app_data::<web::Data<TokenManager>>() {
        Some(tm) => tm.clone(),
        None => {
            return Err((
                AuthError::Internal("TokenManager not configured".to_string()).into(),
                req,
            ))
        }
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
