//! Authentication middleware for Actix-Web

use super::error::AuthError;
use super::manager::TokenManager;
use super::token::{ApiToken, Permission};
use crate::api::config::AuthEnabled;
use actix_web::dev::ServiceRequest;
use actix_web::web;
use actix_web::Error;
use actix_web::HttpMessage;
use actix_web::HttpRequest;
use actix_web::HttpResponse;
use actix_web::ResponseError;
use actix_web_httpauth::extractors::bearer::BearerAuth;

/// Authenticate requests after checking explicit disablement and the public probe.
///
/// Use `HttpAuthentication::with_fn(request_validator)` so missing credentials
/// reach this policy instead of being rejected by the mandatory Bearer extractor.
/// Only GET on the exact liveness path is public; other health endpoints expose
/// engine details and retain authentication.
pub async fn request_validator(
    req: ServiceRequest,
    credentials: Option<BearerAuth>,
) -> Result<ServiceRequest, (Error, ServiceRequest)> {
    if (req.method() == actix_web::http::Method::GET && req.path() == "/health/liveness")
        || req
            .app_data::<web::Data<AuthEnabled>>()
            .is_some_and(|flag| !flag.get_ref().0)
    {
        return Ok(req);
    }
    match credentials {
        Some(credentials) => bearer_validator(req, credentials).await,
        None => Err((AuthError::MissingToken.into(), req)),
    }
}

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
        .app_data::<web::Data<AuthEnabled>>()
        .map(|flag| flag.get_ref().0)
        .unwrap_or(true);

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
/// When authentication is disabled, all requests pass through.
/// When enabled, checks that the authenticated token has the required
/// permission. Returns `AuthError::InsufficientPermissions` as an HTTP
/// response if the token does not have the required permission.
///
/// Call this at the top of any handler that needs permission control:
/// ```ignore
/// if let Err(resp) = require_permission(&req, Permission::Read) {
///     return *resp;
/// }
/// ```
pub fn require_permission(
    req: &HttpRequest,
    expected: Permission,
) -> Result<(), Box<HttpResponse>> {
    // Check if auth is enabled via the flag stored in app_data by start_server
    let auth_enabled = req
        .app_data::<web::Data<AuthEnabled>>()
        .map(|flag| flag.get_ref().0)
        .unwrap_or(true);

    if !auth_enabled {
        return Ok(());
    }

    match req.extensions().get::<ApiToken>() {
        Some(token) => {
            if token.has_permission(expected) {
                Ok(())
            } else {
                Err(Box::new(
                    AuthError::InsufficientPermissions.error_response(),
                ))
            }
        }
        None => Err(Box::new(
            AuthError::InsufficientPermissions.error_response(),
        )),
    }
}
