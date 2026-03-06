//! Actix-web middleware for Bearer token authentication

use super::manager::TokenManager;
use super::token::ApiToken;
use actix_web::dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::error::ErrorUnauthorized;
use actix_web::{Error, HttpMessage};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use futures_util::future::{ready, LocalBoxFuture, Ready};
use std::rc::Rc;

pub struct BearerAuth {
    token_manager: Rc<TokenManager>,
}

impl BearerAuth {
    pub fn new(token_manager: Rc<TokenManager>) -> Self {
        Self { token_manager }
    }
}

impl<S, B> Transform<S, ServiceRequest> for BearerAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = BearerAuthMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(BearerAuthMiddleware {
            service: Rc::new(service),
            token_manager: self.token_manager.clone(),
        }))
    }
}

pub struct BearerAuthMiddleware<S> {
    service: Rc<S>,
    token_manager: Rc<TokenManager>,
}

impl<S, B> Service<ServiceRequest> for BearerAuthMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let token_manager = self.token_manager.clone();

        Box::pin(async move {
            let auth = match BearerAuth::extract(&req).await {
                Ok(auth) => auth,
                Err(_) => return Err(ErrorUnauthorized("Missing or invalid Bearer token")),
            };

            let token = match token_manager.validate_token(auth.token()) {
                Ok(t) => t,
                Err(e) => return Err(ErrorUnauthorized(format!("Invalid token: {}", e))),
            };

            req.extensions_mut().insert(token);
            service.call(req).await
        })
    }
}

/// Helper to extract token from request extensions
pub fn bearer_validator(token: ApiToken) -> Result<ApiToken, Error> {
    Ok(token)
}
