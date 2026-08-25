//! Authentication middleware for Actix-Web

use super::error::AuthError;
use super::manager::TokenManager;
use super::token::{ApiToken, Permission};
use super::AuthEnabled;
use actix_web::dev::ServiceRequest;
use actix_web::web;
use actix_web::Error;
use actix_web::HttpMessage;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use actix_web::ResponseError;
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
    // Check if auth is enabled via the flag stored in app_data by start_server.
    // A missing flag means the server was wired incorrectly; fail closed rather
    // than silently letting every request through.
    let auth_enabled = match req.app_data::<web::Data<AuthEnabled>>() {
        Some(flag) => flag.is_enabled(),
        None => {
            return Err((
                AuthError::Internal("AuthEnabled flag not configured".to_string()).into(),
                req,
            ))
        }
    };

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

/// Require a specific permission for the current request.
///
/// When authentication is disabled, all requests pass through. When the
/// [`AuthEnabled`] flag is missing from app data the request is rejected,
/// because a misconfigured server must not silently grant access.
/// When enabled, checks that the authenticated token has the required
/// permission. Returns `AuthError::InsufficientPermissions` as an HTTP
/// response if the token does not have the required permission.
///
/// Call this at the top of any handler that needs permission control:
/// ```ignore
/// if let Err(resp) = require_permission(&req, Permission::Read) {
///     return resp;
/// }
/// ```
pub fn require_permission(req: &HttpRequest, expected: Permission) -> Result<(), HttpResponse> {
    // Check if auth is enabled via the flag stored in app_data by start_server.
    // A missing flag means the server was wired incorrectly; fail closed rather
    // than granting every permission.
    let auth_enabled = match req.app_data::<web::Data<AuthEnabled>>() {
        Some(flag) => flag.is_enabled(),
        None => return Err(AuthError::InsufficientPermissions.error_response()),
    };

    if !auth_enabled {
        return Ok(());
    }

    match req.extensions().get::<ApiToken>() {
        Some(token) => {
            if token.has_permission(expected) {
                Ok(())
            } else {
                Err(AuthError::InsufficientPermissions.error_response())
            }
        }
        None => Err(AuthError::InsufficientPermissions.error_response()),
    }
}
