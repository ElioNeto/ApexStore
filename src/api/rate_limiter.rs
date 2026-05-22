//! Simple IP-based rate limiting middleware.
//!
//! Tracks request frequency per client IP address using a sliding window.
//! When a client exceeds the allowed requests per minute, subsequent
//! requests receive a `429 Too Many Requests` response.
//!
//! Supports per-endpoint rate limits and per-IP tracking with configurable
//! limits for observability.

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::web::Data;
use actix_web::Error;
use serde::Serialize;
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Per-IP rate tracking entry.
#[derive(Debug, Clone)]
struct IpTrack {
    /// Timestamps of recent requests (sliding window).
    timestamps: Vec<Instant>,
    /// Per-endpoint counters for this IP.
    endpoint_counts: HashMap<String, usize>,
}

impl IpTrack {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
            endpoint_counts: HashMap::new(),
        }
    }

    fn prune(&mut self, window: Duration) {
        let now = Instant::now();
        self.timestamps.retain(|t| now.duration_since(*t) < window);
        // endpoint_counts are pruned implicitly when the whole IpTrack
        // is removed (retain below checks timestamps.is_empty()).
    }
}

/// Shared state for rate limiting, tracked across all worker threads.
pub struct RateLimiterState {
    requests: Mutex<HashMap<SocketAddr, IpTrack>>,
    max_requests_per_minute: usize,
    /// Per-endpoint rate limits (requests per minute). Empty = use global default.
    endpoint_limits: HashMap<String, usize>,
}

impl RateLimiterState {
    pub fn new(max_requests_per_minute: usize) -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            max_requests_per_minute,
            endpoint_limits: HashMap::new(),
        }
    }

    /// Set a per-endpoint rate limit.
    ///
    /// `endpoint` is the URL path pattern (e.g., "/keys", "/admin/compact").
    /// When set, requests to that path use this limit instead of the global default.
    pub fn set_endpoint_limit(&mut self, endpoint: &str, limit: usize) {
        self.endpoint_limits.insert(endpoint.to_string(), limit);
    }

    /// Get the effective limit for a given endpoint.
    fn effective_limit(&self, endpoint: &str) -> usize {
        self.endpoint_limits
            .get(endpoint)
            .copied()
            .unwrap_or(self.max_requests_per_minute)
    }

    fn is_rate_limited(&self, peer: SocketAddr, endpoint: Option<&str>) -> bool {
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let limit = match endpoint {
            Some(ep) => self.effective_limit(ep),
            None => self.max_requests_per_minute,
        };

        if limit == 0 {
            return false; // No limit = disabled
        }

        let mut requests = self.requests.lock().expect("rate limiter lock poisoned");
        // Prune all entries
        requests.retain(|_, track| {
            track.prune(window);
            !track.timestamps.is_empty()
        });

        let track = requests.entry(peer).or_insert_with(IpTrack::new);

        // Per-endpoint limit: use dedicated endpoint counter
        if let Some(ep) = endpoint {
            let count = track.endpoint_counts.get(ep).copied().unwrap_or(0);
            if count >= limit {
                return true;
            }
            track.timestamps.push(now);
            *track.endpoint_counts.entry(ep.to_string()).or_insert(0) += 1;
            return false;
        }

        // Global per-IP limit: use total timestamp count
        if track.timestamps.len() >= limit {
            return true;
        }
        track.timestamps.push(now);
        false
    }

    /// Get current state summary for all tracked IPs.
    pub fn get_state(&self) -> RateLimitSummary {
        let requests = self.requests.lock().expect("rate limiter lock poisoned");
        let mut ips = Vec::new();
        for (addr, track) in requests.iter() {
            ips.push(IpSummary {
                ip: addr.to_string(),
                request_count: track.timestamps.len(),
                endpoint_counts: track.endpoint_counts.clone(),
            });
        }
        RateLimitSummary {
            global_limit: self.max_requests_per_minute,
            endpoint_limits: self.endpoint_limits.clone(),
            tracked_ips: ips,
        }
    }
}

/// Summary of current rate limiter state.
#[derive(Debug, Clone, Serialize)]
pub struct RateLimitSummary {
    pub global_limit: usize,
    pub endpoint_limits: HashMap<String, usize>,
    pub tracked_ips: Vec<IpSummary>,
}

/// Per-IP summary.
#[derive(Debug, Clone, Serialize)]
pub struct IpSummary {
    pub ip: String,
    pub request_count: usize,
    pub endpoint_counts: HashMap<String, usize>,
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
        if let Some(state) = req.app_data::<Data<RateLimiterState>>() {
            if state.max_requests_per_minute > 0 {
                if let Some(peer) = req.peer_addr() {
                    // Extract endpoint path for per-endpoint rate limiting
                    let endpoint = req.path().to_string();
                    if state.is_rate_limited(peer, Some(&endpoint)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_basic() {
        let state = RateLimiterState::new(3);
        let peer: SocketAddr = "127.0.0.1:12345".parse().unwrap();

        // First 3 requests should not be rate limited
        assert!(!state.is_rate_limited(peer, None));
        assert!(!state.is_rate_limited(peer, None));
        assert!(!state.is_rate_limited(peer, None));
        // 4th should be limited
        assert!(state.is_rate_limited(peer, None));
    }

    #[test]
    fn test_per_endpoint_limit() {
        let mut state = RateLimiterState::new(10);
        state.set_endpoint_limit("/admin/compact", 2);

        let peer: SocketAddr = "127.0.0.1:54321".parse().unwrap();

        // Global route: should use limit 10
        assert!(!state.is_rate_limited(peer, Some("/keys")));

        // Admin route: limit is 2
        assert!(!state.is_rate_limited(peer, Some("/admin/compact")));
        assert!(!state.is_rate_limited(peer, Some("/admin/compact")));
        assert!(state.is_rate_limited(peer, Some("/admin/compact")));
    }

    #[test]
    fn test_zero_limit_disabled() {
        let state = RateLimiterState::new(0);
        let peer: SocketAddr = "127.0.0.1:9999".parse().unwrap();
        // Zero = disabled, never limited
        for _ in 0..100 {
            assert!(!state.is_rate_limited(peer, None));
        }
    }

    #[test]
    fn test_get_state() {
        let state = RateLimiterState::new(5);
        let peer: SocketAddr = "10.0.0.1:8080".parse().unwrap();
        state.is_rate_limited(peer, Some("/keys"));

        let summary = state.get_state();
        assert_eq!(summary.global_limit, 5);
        assert_eq!(summary.tracked_ips.len(), 1);
        assert_eq!(summary.tracked_ips[0].ip, "10.0.0.1:8080");
    }
}
