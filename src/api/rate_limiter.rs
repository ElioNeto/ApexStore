//! Simple IP-based rate limiting middleware.
//!
//! Tracks request frequency per client IP address using a sliding window.
//! When a client exceeds the allowed requests per minute, subsequent
//! requests receive a `429 Too Many Requests` response.

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::Error;
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Shared state for rate limiting, tracked across all worker threads.
pub struct RateLimiterState {
    requests: Mutex<HashMap<SocketAddr, Vec<Instant>>>,
    max_requests_per_minute: usize,
}

impl RateLimiterState {
    pub fn new(max_requests_per_minute: usize) -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            max_requests_per_minute,
        }
    }

    fn is_rate_limited(&self, peer: SocketAddr) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut requests = self.requests.lock().expect("rate limiter lock poisoned");
        requests.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
        let timestamps = requests.entry(peer).or_default();
        if timestamps.len() >= self.max_requests_per_minute {
            return true;
        }
        timestamps.push(now);
        false
    }
}

/// Rate limiter middleware factory.
pub struct RateLimiter;

/// Inner middleware service wrapping the next service in the chain.
pub struct RateLimiterMiddleware<S> {
    service: S,
}

impl<S, B> Transform<S, ServiceRequest> for RateLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Transform = RateLimiterMiddleware<S>;
    type InitError = ();
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RateLimiterMiddleware { service }))
    }
}

impl<S, B> Service<ServiceRequest> for RateLimiterMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
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
        if let Some(state) = req.app_data::<actix_web::web::Data<RateLimiterState>>() {
            if state.max_requests_per_minute > 0 {
                if let Some(peer) = req.peer_addr() {
                    if state.is_rate_limited(peer) {
                        return Box::pin(ready(Err(
                            actix_web::error::ErrorTooManyRequests("rate limit exceeded"),
                        )));
                    }
                }
            }
        }
        Box::pin(self.service.call(req))
    }
}
