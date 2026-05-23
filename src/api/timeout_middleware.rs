//! Request timeout middleware for actix-web.
//!
//! Wraps every request with an upper time limit. If the request handler does
//! not complete within the timeout, a `408 Request Timeout` response is
//! returned.
//!
//! The default timeout is read from the `REQUEST_TIMEOUT_SECONDS` environment
//! variable (default: 30).

use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use std::env;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::timeout;

/// Middleware factory that applies a timeout to every request.
pub struct RequestTimeout;

/// Middleware service wrapping the inner service with a timeout.
pub struct RequestTimeoutMiddleware<S> {
    service: S,
    timeout_duration: Duration,
}

impl<S, B> Transform<S, ServiceRequest> for RequestTimeout
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestTimeoutMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        let timeout_secs = env::var("REQUEST_TIMEOUT_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);

        ready(Ok(RequestTimeoutMiddleware {
            service,
            timeout_duration: Duration::from_secs(timeout_secs),
        }))
    }
}

impl<S, B> actix_web::dev::Service<ServiceRequest> for RequestTimeoutMiddleware<S>
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
        let fut = self.service.call(req);
        let duration = self.timeout_duration;

        Box::pin(async move {
            match timeout(duration, fut).await {
                Ok(result) => result,
                Err(_elapsed) => {
                    // Return a 408 error using actix-web's error type system,
                    // which actix-web converts into a proper error response.
                    Err(actix_web::error::InternalError::from_response(
                        "request timed out",
                        HttpResponse::RequestTimeout()
                            .content_type("application/json")
                            .body(
                                serde_json::json!({
                                    "error": "request timed out",
                                    "timeout_seconds": duration.as_secs()
                                })
                                .to_string(),
                            ),
                    )
                    .into())
                }
            }
        })
    }
}
