//! Audit logging middleware for actix-web.
//!
//! Records every request along with the authenticated principal, client IP, HTTP method,
//! path, response status code, and processing duration. Must be placed **after**
//! the authentication middleware so that the `ApiToken` (with the principal's
//! name) is available in request extensions.
//!
//! # Log format
//!
//! All events are written via `tracing::info!` with `target: "apexstore::audit"`
//! so that they can be routed to a dedicated audit sink via an `EnvFilter` or
//! a custom tracing subscriber layer, independently of regular application logs.

use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use std::{
    future::{ready, Ready},
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use crate::api::{auth::token::ApiToken, rate_limiter::get_client_ip};

/// Middleware factory that records audit events for every request.
pub struct AuditMiddleware;

/// Middleware service wrapping the inner service with audit logging.
pub struct AuditMiddlewareService<S> {
    service: S,
}

impl<S, B> Transform<S, ServiceRequest> for AuditMiddleware
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = AuditMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuditMiddlewareService { service }))
    }
}

impl<S, B> actix_web::dev::Service<ServiceRequest> for AuditMiddlewareService<S>
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let start = Instant::now();
        // Use the same identity policy as rate limiting, never a raw forwarding
        // header or ConnectionInfo's unverified realip_remote_addr().
        let client_ip = get_client_ip(&req)
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let method = req.method().to_string();
        let path = req.path().to_string();
        let query_string = req.query_string().to_string();

        // Extract the authenticated principal from request extensions.
        // The HttpAuthentication::bearer middleware (which runs before us in the
        // outer layers) stores the ApiToken when validation succeeds.
        // If auth is disabled or validation fails, the principal is "anonymous".
        let principal = req
            .extensions()
            .get::<ApiToken>()
            .map(|token| token.name.clone())
            .unwrap_or_else(|| "anonymous".to_string());

        let fut = self.service.call(req);

        Box::pin(async move {
            let result = fut.await;
            let duration_us = start.elapsed().as_micros() as u64;

            match &result {
                Ok(res) => {
                    let status = res.status().as_u16();
                    tracing::info!(
                        target: "apexstore::audit",
                        method = %method,
                        path = %path,
                        query = %query_string,
                        status = status,
                        principal = %principal,
                        client_ip = %client_ip,
                        duration_us = duration_us,
                        "audit event"
                    );
                }
                Err(err) => {
                    // Middleware errors carry their actual HTTP status, including
                    // admission refusals such as 429; they are not all server errors.
                    tracing::info!(
                        target: "apexstore::audit",
                        method = %method,
                        path = %path,
                        query = %query_string,
                        status = err.as_response_error().status_code().as_u16(),
                        principal = %principal,
                        client_ip = %client_ip,
                        duration_us = duration_us,
                        error = %err,
                        "audit event (error)"
                    );
                }
            }

            result
        })
    }
}
