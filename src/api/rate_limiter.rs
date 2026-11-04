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
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::future::{ready, Ready};
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

/// Per-IP rate tracking entry.
#[derive(Debug, Clone)]
struct IpTrack {
    /// Timestamps of recent requests (sliding window).
    timestamps: VecDeque<Instant>,
    /// Accepted requests per registered route in the same sliding window.
    endpoint_timestamps: HashMap<String, VecDeque<Instant>>,
}

impl IpTrack {
    fn new() -> Self {
        Self {
            timestamps: VecDeque::new(),
            endpoint_timestamps: HashMap::new(),
        }
    }

    fn prune(&mut self, now: Instant, window: Duration) {
        prune_timestamps(&mut self.timestamps, now, window);
        self.endpoint_timestamps.retain(|_, timestamps| {
            prune_timestamps(timestamps, now, window);
            !timestamps.is_empty()
        });
    }
}

fn prune_timestamps(timestamps: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while timestamps
        .front()
        .is_some_and(|timestamp| now.saturating_duration_since(*timestamp) >= window)
    {
        timestamps.pop_front();
    }
}

const WINDOW: Duration = Duration::from_secs(60);

struct RateLimitShard {
    requests: HashMap<IpAddr, IpTrack>,
    last_cleanup: Instant,
}

impl RateLimitShard {
    fn prune(&mut self, now: Instant) {
        self.requests.retain(|_, track| {
            track.prune(now, WINDOW);
            !track.timestamps.is_empty()
        });
        self.last_cleanup = now;
    }
}

/// Number of shards for the rate limiter's IP tracking map.
/// Higher values reduce lock contention under high concurrency.
const NUM_SHARDS: usize = 16;

/// Shared state for rate limiting, tracked across all worker threads.
pub struct RateLimiterState {
    shards: Vec<Mutex<RateLimitShard>>,
    max_requests_per_minute: usize,
    /// Additional per-route limits; zero disables only that route's extra cap.
    endpoint_limits: HashMap<String, usize>,
}

impl RateLimiterState {
    pub fn new(max_requests_per_minute: usize) -> Self {
        let mut shards = Vec::with_capacity(NUM_SHARDS);
        for _ in 0..NUM_SHARDS {
            shards.push(Mutex::new(RateLimitShard {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            }));
        }
        Self {
            shards,
            max_requests_per_minute,
            endpoint_limits: HashMap::new(),
        }
    }

    /// Select the shard for a given peer IP address by hashing the address.
    fn shard_for(&self, peer: IpAddr) -> &Mutex<RateLimitShard> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        peer.hash(&mut hasher);
        let idx = hasher.finish() as usize % NUM_SHARDS;
        &self.shards[idx]
    }

    /// Set a per-endpoint rate limit.
    ///
    /// `endpoint` is the registered route pattern (e.g., "/keys/{key}").
    /// This cap and the global per-IP cap apply independently. Zero disables
    /// only this endpoint's additional cap, never the global one.
    pub fn set_endpoint_limit(&mut self, endpoint: &str, limit: usize) {
        self.endpoint_limits.insert(endpoint.to_string(), limit);
    }

    fn is_rate_limited(&self, peer: IpAddr, endpoint: Option<&str>) -> bool {
        self.is_rate_limited_with_clock(peer, endpoint, Instant::now)
    }

    fn is_rate_limited_with_clock(
        &self,
        peer: IpAddr,
        endpoint: Option<&str>,
        clock: impl FnOnce() -> Instant,
    ) -> bool {
        let endpoint_limit = endpoint
            .and_then(|route| self.endpoint_limits.get(route))
            .copied()
            .unwrap_or(0);
        if self.max_requests_per_minute == 0 && endpoint_limit == 0 {
            return false;
        }

        let mut shard = self.shard_for(peer).lock();
        // Read the clock under the lock so concurrent requests append in order.
        let now = clock();
        // Sweep inactive peers at most once per window, rather than on every
        // request. The touched peer is always pruned before enforcing its caps.
        if now.saturating_duration_since(shard.last_cleanup) >= WINDOW {
            shard.prune(now);
        }
        let track = shard.requests.entry(peer).or_insert_with(IpTrack::new);
        track.prune(now, WINDOW);
        if self.max_requests_per_minute > 0
            && track.timestamps.len() >= self.max_requests_per_minute
        {
            return true;
        }
        if let Some(route) = endpoint {
            if endpoint_limit > 0
                && track
                    .endpoint_timestamps
                    .get(route)
                    .is_some_and(|timestamps| timestamps.len() >= endpoint_limit)
            {
                return true;
            }
            track
                .endpoint_timestamps
                .entry(route.to_owned())
                .or_default()
                .push_back(now);
        }
        track.timestamps.push_back(now);
        false
    }

    /// Accepted requests tracked in the last 60 seconds. Requests with both
    /// caps disabled and rejected requests are not counted. Endpoint counts
    /// use the same window as the per-IP total.
    pub fn get_state(&self) -> RateLimitSummary {
        self.get_state_with_clock(Instant::now)
    }

    fn get_state_with_clock(&self, clock: impl Fn() -> Instant) -> RateLimitSummary {
        let mut ips = Vec::new();
        for shard in &self.shards {
            let mut shard = shard.lock();
            shard.prune(clock());
            for (addr, track) in &shard.requests {
                ips.push(IpSummary {
                    ip: addr.to_string(),
                    request_count: track.timestamps.len(),
                    endpoint_counts: track
                        .endpoint_timestamps
                        .iter()
                        .map(|(route, timestamps)| (route.clone(), timestamps.len()))
                        .collect(),
                });
            }
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
            if let Some(peer) = get_client_ip(&req) {
                // App middleware runs before routing sets match_pattern().
                // Resolve against the router's map, including scope prefixes.
                let endpoint = req.resource_map().match_pattern(req.path());
                if state.is_rate_limited(peer, Some(endpoint.as_deref().unwrap_or("<unmatched>"))) {
                    return Box::pin(ready(Err(actix_web::error::ErrorTooManyRequests(
                        "rate limit exceeded",
                    ))));
                }
            }
        }
        Box::pin(self.service.call(req))
    }
}

/// Extract the client IP address from a request.
///
/// Use the transport peer as the identity until a trusted-proxy policy is
/// explicitly configured. Forwarding headers are attacker-controlled and must
/// never create a fresh bucket or erase an otherwise known peer address.
pub fn get_client_ip(req: &ServiceRequest) -> Option<IpAddr> {
    req.peer_addr().map(|s| s.ip())
}

/// Parse the first IP address from an `X-Forwarded-For` header value.
///
/// The header may contain a comma-separated list of IP addresses; this function
/// returns only the first (leftmost) one. This is a parsing utility: the result
/// is untrusted and must not be used as an authenticated client identity.
///
/// Returns `None` when the value is empty, unparseable, or contains no valid IP.
pub fn parse_x_forwarded_for(value: &str) -> Option<IpAddr> {
    value
        .split(',')
        .next()
        .map(str::trim)
        .and_then(|s| s.parse::<IpAddr>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_window_expires_while_another_endpoint_stays_active() {
        let mut state = RateLimiterState::new(100);
        state.set_endpoint_limit("/admin/compact", 2);
        let peer = "127.0.0.1".parse().unwrap();
        let start = Instant::now();
        let request = |route, seconds| {
            state.is_rate_limited_with_clock(peer, Some(route), || {
                start + Duration::from_secs(seconds)
            })
        };
        assert!(!request("/admin/compact", 0));
        assert!(!request("/admin/compact", 1));
        assert!(request("/admin/compact", 2));
        assert!(!request("/health/liveness", 30));
        assert!(request("/admin/compact", 59));
        assert!(!request("/admin/compact", 60));
        assert!(request("/admin/compact", 60));
        assert!(!request("/admin/compact", 61));
    }

    #[test]
    fn global_limit_applies_across_endpoints() {
        let state = RateLimiterState::new(2);
        let peer = "127.0.0.1".parse().unwrap();
        assert!(!state.is_rate_limited(peer, Some("/keys")));
        assert!(!state.is_rate_limited(peer, Some("/stats")));
        assert!(state.is_rate_limited(peer, Some("/health/liveness")));
    }

    #[test]
    fn zero_endpoint_limit_does_not_disable_the_global_limit() {
        let mut state = RateLimiterState::new(1);
        state.set_endpoint_limit("/keys", 0);
        let peer = "127.0.0.1".parse().unwrap();
        assert!(!state.is_rate_limited(peer, Some("/keys")));
        assert!(state.is_rate_limited(peer, Some("/keys")));
    }

    #[test]
    fn summary_counts_only_accepted_requests_in_the_current_window() {
        let mut state = RateLimiterState::new(10);
        state.set_endpoint_limit("/admin/compact", 1);
        let peer = "127.0.0.1".parse().unwrap();
        let start = Instant::now();
        assert!(!state.is_rate_limited_with_clock(peer, Some("/admin/compact"), || start));
        assert!(
            state.is_rate_limited_with_clock(peer, Some("/admin/compact"), || {
                start + Duration::from_secs(20)
            })
        );
        assert!(!state.is_rate_limited_with_clock(peer, Some("/keys"), || {
            start + Duration::from_secs(30)
        }));
        let summary = state.get_state_with_clock(|| start + Duration::from_secs(59));
        assert_eq!(summary.tracked_ips[0].request_count, 2);
        assert_eq!(summary.tracked_ips[0].endpoint_counts["/admin/compact"], 1);
        assert_eq!(summary.tracked_ips[0].endpoint_counts["/keys"], 1);
        let summary = state.get_state_with_clock(|| start + Duration::from_secs(60));
        assert_eq!(summary.tracked_ips[0].request_count, 1);
        assert_eq!(summary.tracked_ips[0].endpoint_counts.len(), 1);
        assert_eq!(summary.tracked_ips[0].endpoint_counts["/keys"], 1);
        assert!(state
            .get_state_with_clock(|| start + Duration::from_secs(90))
            .tracked_ips
            .is_empty());
    }

    #[test]
    fn concurrent_requests_share_a_cap_but_different_peers_do_not() {
        let state = RateLimiterState::new(7);
        let peer = "127.0.0.1".parse().unwrap();
        let barrier = std::sync::Barrier::new(20);
        let accepted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..20)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        !state.is_rate_limited(peer, Some("/keys"))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(accepted, 7);
        assert_eq!(state.get_state().tracked_ips[0].request_count, 7);
        assert!(!state.is_rate_limited("127.0.0.2".parse().unwrap(), Some("/keys")));
    }

    #[test]
    fn test_rate_limiter_basic() {
        let state = RateLimiterState::new(3);
        let peer: IpAddr = "127.0.0.1".parse().unwrap();

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

        let peer: IpAddr = "127.0.0.1".parse().unwrap();

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
        let peer: IpAddr = "127.0.0.1".parse().unwrap();
        // Zero = disabled, never limited
        for _ in 0..100 {
            assert!(!state.is_rate_limited(peer, None));
        }
    }

    #[test]
    fn test_get_state() {
        let state = RateLimiterState::new(5);
        let peer: IpAddr = "10.0.0.1".parse().unwrap();
        state.is_rate_limited(peer, Some("/keys"));

        let summary = state.get_state();
        assert_eq!(summary.global_limit, 5);
        assert_eq!(summary.tracked_ips.len(), 1);
        assert_eq!(summary.tracked_ips[0].ip, "10.0.0.1");
    }

    // ── parse_x_forwarded_for tests ────────────────────────────────────────

    #[test]
    fn test_parse_xff_single_ipv4() {
        assert_eq!(
            parse_x_forwarded_for("203.0.113.195"),
            Some("203.0.113.195".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_parse_xff_multiple_ips() {
        // Only the first IP is returned
        assert_eq!(
            parse_x_forwarded_for("203.0.113.195, 198.51.100.42, 192.0.2.1"),
            Some("203.0.113.195".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_parse_xff_ipv6() {
        assert_eq!(
            parse_x_forwarded_for("2001:db8::1"),
            Some("2001:db8::1".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn test_parse_xff_invalid() {
        assert_eq!(parse_x_forwarded_for("not-an-ip"), None);
    }

    #[test]
    fn test_parse_xff_empty() {
        assert_eq!(parse_x_forwarded_for(""), None);
    }

    #[test]
    fn test_parse_xff_with_trailing_comma() {
        assert_eq!(
            parse_x_forwarded_for("203.0.113.195, "),
            Some("203.0.113.195".parse::<IpAddr>().unwrap())
        );
    }
}
