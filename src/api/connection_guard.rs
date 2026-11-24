//! Per-IP concurrent connection limiter.
//!
//! Tracks the number of active (in-flight) requests per client IP address.
//! When a client exceeds [`MAX_CONNECTIONS_PER_IP`] concurrent requests,
//! subsequent requests receive a `429 Too Many Requests` response.
//!
//! This is distinct from rate limiting — it limits *concurrency* rather than
//! *frequency*.

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceRequest, ServiceResponse, Transform};
use actix_web::web::Data;
use actix_web::Error;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

/// Default maximum number of concurrent requests allowed per IP address.
pub const MAX_CONNECTIONS_PER_IP: usize = 100;

/// Default maximum distinct IPs with in-flight requests, shared by all workers.
pub const DEFAULT_MAX_TRACKED_IPS: usize = 16_384;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionLimitSummary {
    pub tracked_ip_count: usize,
    pub max_tracked_ips: usize,
    pub active_requests: usize,
    pub max_per_ip: usize,
    pub capacity_rejections: u64,
}

/// Guard that tracks per-IP concurrent connections.
///
/// Call [`try_acquire`](Self::try_acquire) before processing a request and
/// [`release`](Self::release) after the response is sent to the client.
pub struct IpConnectionGuard {
    connections: Mutex<HashMap<String, usize>>,
    /// Maximum number of concurrent requests allowed per IP.
    max_per_ip: usize,
    max_tracked_ips: usize,
    capacity_rejections: AtomicU64,
}

impl IpConnectionGuard {
    /// Create a new empty guard with the given per-IP limit.
    pub fn new(max_per_ip: usize) -> Self {
        Self::with_capacity(max_per_ip, DEFAULT_MAX_TRACKED_IPS)
    }

    /// Refuse new peers at capacity without evicting live connection counts.
    /// Zero capacity rejects every new peer.
    pub fn with_capacity(max_per_ip: usize, max_tracked_ips: usize) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            max_per_ip,
            max_tracked_ips,
            capacity_rejections: AtomicU64::new(0),
        }
    }

    /// Try to acquire a connection slot for `ip`.
    ///
    /// Returns `true` if the IP is allowed (under the configured limit), `false`
    /// if the connection limit has been reached.
    pub fn try_acquire(&self, ip: &str) -> bool {
        self.try_acquire_with_max(ip, self.max_per_ip)
    }

    /// Try to acquire a connection slot for `ip` with an explicit max (used by tests).
    pub fn try_acquire_with_max(&self, ip: &str, max_per_ip: usize) -> bool {
        let mut map = self.connections.lock();
        if max_per_ip == 0 {
            return false;
        }
        if let Some(count) = map.get_mut(ip) {
            if *count >= max_per_ip {
                return false;
            }
            *count += 1;
            return true;
        }
        if map.len() >= self.max_tracked_ips {
            self.capacity_rejections.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        map.insert(ip.to_owned(), 1);
        true
    }

    /// Snapshot of occupied slots and capacity-pressure rejections.
    pub fn get_state(&self) -> ConnectionLimitSummary {
        let map = self.connections.lock();
        ConnectionLimitSummary {
            tracked_ip_count: map.len(),
            max_tracked_ips: self.max_tracked_ips,
            active_requests: map.values().copied().fold(0, usize::saturating_add),
            max_per_ip: self.max_per_ip,
            capacity_rejections: self.capacity_rejections.load(Ordering::Relaxed),
        }
    }

    /// Returns the configured per-IP limit.
    pub fn max_per_ip(&self) -> usize {
        self.max_per_ip
    }

    /// Release a connection slot for `ip`.
    ///
    /// Must be called exactly once for every successful [`try_acquire`](Self::try_acquire).
    pub fn release(&self, ip: &str) {
        let mut map = self.connections.lock();
        if let Some(count) = map.get_mut(ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(ip);
            }
        }
    }
}

/// Releases exactly one acquired slot on completion, cancellation or unwinding.
struct ConnectionPermit {
    guard: Data<IpConnectionGuard>,
    ip: String,
}

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.guard.release(&self.ip);
    }
}

// ── Middleware ───────────────────────────────────────────────────────────────

/// Middleware factory that limits concurrent connections per IP address.
pub struct ConnectionLimiter;

impl<S, B> Transform<S, ServiceRequest> for ConnectionLimiter
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = ConnectionLimiterMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ConnectionLimiterMiddleware { service }))
    }
}

/// Middleware service that enforces the per-IP connection limit.
pub struct ConnectionLimiterMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for ConnectionLimiterMiddleware<S>
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
        let permit = if let Some(guard) = req.app_data::<Data<IpConnectionGuard>>() {
            let ip = req
                .peer_addr()
                .map(|s| s.ip().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            if !guard.try_acquire(&ip) {
                return Box::pin(ready(Err(actix_web::error::ErrorTooManyRequests(
                    "too many concurrent connections from this IP",
                ))));
            }
            Some(ConnectionPermit {
                guard: guard.clone(),
                ip,
            })
        } else {
            None
        };

        // Acquire before calling the inner service so a synchronous panic also
        // drops the permit. The future owns it even before its first poll.
        let fut = self.service.call(req);
        Box::pin(async move {
            let _permit = permit;
            fut.await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_capacity_is_atomic_and_existing_peers_keep_their_slots() {
        let guard = IpConnectionGuard::with_capacity(2, 3);
        let barrier = std::sync::Barrier::new(20);
        let accepted = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..20)
                .map(|i| {
                    let guard = &guard;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        guard.try_acquire(&format!("peer-{i}"))
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| usize::from(h.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(accepted, 3);
        let peers: Vec<_> = guard.connections.lock().keys().cloned().collect();
        for peer in &peers {
            assert!(guard.try_acquire(peer));
            assert!(!guard.try_acquire(peer));
        }
        let summary = guard.get_state();
        assert_eq!(summary.tracked_ip_count, 3);
        assert_eq!(summary.active_requests, 6);
        assert_eq!(summary.capacity_rejections, 17);
        guard.release(&peers[0]);
        assert!(!guard.try_acquire("new-peer"));
        guard.release(&peers[0]);
        assert!(guard.try_acquire("new-peer"));
        let zero = IpConnectionGuard::with_capacity(1, 0);
        assert!(!zero.try_acquire("peer"));
        assert_eq!(zero.get_state().tracked_ip_count, 0);
    }

    #[actix_web::test]
    async fn tracking_capacity_middleware_frees_admission_after_cancellation() {
        let guard = Data::new(IpConnectionGuard::with_capacity(2, 1));
        let service = ConnectionLimiter
            .new_transform(actix_web::dev::fn_service(
                |req: ServiceRequest| async move {
                    std::future::pending::<()>().await;
                    Ok::<_, Error>(req.into_response(actix_web::HttpResponse::Ok().finish()))
                },
            ))
            .await
            .unwrap();
        let request = |ip: &str| {
            actix_web::test::TestRequest::default()
                .peer_addr(ip.parse().unwrap())
                .app_data(guard.clone())
                .to_srv_request()
        };
        let first = service.call(request("192.0.2.1:1"));
        let same_peer = service.call(request("192.0.2.1:2"));
        let rejected = service.call(request("192.0.2.2:1")).await.unwrap_err();
        assert_eq!(
            rejected.as_response_error().status_code(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
        drop(first);
        assert!(service.call(request("192.0.2.2:1")).await.is_err());
        drop(same_peer);
        let admitted = service.call(request("192.0.2.2:1"));
        assert_eq!(guard.get_state().tracked_ip_count, 1);
        assert_eq!(guard.get_state().active_requests, 1);
        assert_eq!(guard.get_state().capacity_rejections, 2);
        drop(admitted);
        assert_eq!(guard.get_state().tracked_ip_count, 0);
    }

    #[test]
    fn tracking_capacity_preserves_live_counts_when_full() {
        let guard = IpConnectionGuard::new(1);
        for ip in 0..16_384 {
            assert!(guard.try_acquire(&format!("peer-{ip}")));
        }
        assert!(
            !guard.try_acquire("overflow"),
            "accepted more than the tracking capacity"
        );
        assert!(!guard.try_acquire("peer-0"), "lost a live connection count");
        guard.release("peer-0");
        assert!(guard.try_acquire("overflow"));
    }

    #[test]
    fn tracking_capacity_rejections_with_zero_limit_do_not_allocate() {
        let guard = IpConnectionGuard::new(0);
        for ip in 0..1000 {
            assert!(!guard.try_acquire(&format!("peer-{ip}")));
        }
        assert!(
            guard.connections.lock().is_empty(),
            "rejected peers leaked map entries"
        );
    }

    #[test]
    fn test_try_acquire_allows_under_limit() {
        let guard = IpConnectionGuard::new(100);
        assert!(guard.try_acquire_with_max("192.168.1.1", 3));
        assert!(guard.try_acquire_with_max("192.168.1.1", 3));
        assert!(guard.try_acquire_with_max("192.168.1.1", 3));
    }

    #[test]
    fn test_try_acquire_rejects_over_limit() {
        let guard = IpConnectionGuard::new(100);
        assert!(guard.try_acquire_with_max("10.0.0.1", 2));
        assert!(guard.try_acquire_with_max("10.0.0.1", 2));
        assert!(!guard.try_acquire_with_max("10.0.0.1", 2));
    }

    #[test]
    fn test_release_frees_slot() {
        let guard = IpConnectionGuard::new(100);
        assert!(guard.try_acquire_with_max("10.0.0.2", 1));
        assert!(!guard.try_acquire_with_max("10.0.0.2", 1));
        guard.release("10.0.0.2");
        assert!(guard.try_acquire_with_max("10.0.0.2", 1));
    }

    #[test]
    fn test_different_ips_independent() {
        let guard = IpConnectionGuard::new(100);
        assert!(guard.try_acquire_with_max("10.0.0.1", 1));
        assert!(guard.try_acquire_with_max("10.0.0.2", 1));
        // Each IP should have its own counter
        assert!(!guard.try_acquire_with_max("10.0.0.1", 1));
        assert!(!guard.try_acquire_with_max("10.0.0.2", 1));
    }

    #[test]
    fn test_release_removes_zero_count() {
        let guard = IpConnectionGuard::new(100);
        guard.try_acquire_with_max("10.0.0.3", 10);
        guard.release("10.0.0.3");
        let map = guard.connections.lock();
        assert!(!map.contains_key("10.0.0.3"));
    }

    #[test]
    fn test_default_max_per_ip() {
        let guard = IpConnectionGuard::new(MAX_CONNECTIONS_PER_IP);
        assert_eq!(guard.max_per_ip(), MAX_CONNECTIONS_PER_IP);
    }

    fn request(guard: &Data<IpConnectionGuard>) -> ServiceRequest {
        actix_web::test::TestRequest::default()
            .peer_addr("192.0.2.1:1234".parse().unwrap())
            .app_data(guard.clone())
            .to_srv_request()
    }

    async fn cancellation_releases_slot(poll_before_drop: bool) {
        let guard = Data::new(IpConnectionGuard::new(1));
        let service = ConnectionLimiter
            .new_transform(actix_web::dev::fn_service(
                |req: ServiceRequest| async move {
                    std::future::pending::<()>().await;
                    Ok::<_, Error>(req.into_response(actix_web::HttpResponse::Ok().finish()))
                },
            ))
            .await
            .unwrap();
        let mut pending = service.call(request(&guard));
        assert!(!guard.try_acquire("192.0.2.1"));
        let rejection = service.call(request(&guard)).await.unwrap_err();
        assert_eq!(
            rejection.as_response_error().status_code(),
            actix_web::http::StatusCode::TOO_MANY_REQUESTS
        );
        assert!(
            !guard.try_acquire("192.0.2.1"),
            "rejecting a request must retain the active permit"
        );
        if poll_before_drop {
            assert!(futures::poll!(pending.as_mut()).is_pending());
        }
        drop(pending);
        assert!(
            guard.try_acquire("192.0.2.1"),
            "cancelled request leaked its connection slot"
        );
        guard.release("192.0.2.1");
    }

    #[actix_web::test]
    async fn dropping_an_unpolled_request_releases_its_connection_slot() {
        cancellation_releases_slot(false).await;
    }

    #[actix_web::test]
    async fn cancelling_an_in_flight_request_releases_its_connection_slot() {
        cancellation_releases_slot(true).await;
    }

    #[actix_web::test]
    async fn synchronous_inner_service_panic_releases_its_connection_slot() {
        let guard = Data::new(IpConnectionGuard::new(1));
        let service = ConnectionLimiter
            .new_transform(actix_web::dev::fn_service(
                |_: ServiceRequest| -> Ready<Result<ServiceResponse, Error>> {
                    panic!("inner call panicked")
                },
            ))
            .await
            .unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(service.call(request(&guard)));
        }));
        assert!(result.is_err());
        assert!(
            guard.try_acquire("192.0.2.1"),
            "synchronous panic leaked its connection slot"
        );
        guard.release("192.0.2.1");
    }

    #[actix_web::test]
    async fn inner_future_panic_releases_its_connection_slot() {
        use futures::FutureExt;
        async fn panicking(_: ServiceRequest) -> Result<ServiceResponse, Error> {
            panic!("inner future panicked");
        }
        let guard = Data::new(IpConnectionGuard::new(1));
        let service = ConnectionLimiter
            .new_transform(actix_web::dev::fn_service(panicking))
            .await
            .unwrap();
        let result = std::panic::AssertUnwindSafe(service.call(request(&guard)))
            .catch_unwind()
            .await;
        assert!(result.is_err());
        assert!(
            guard.try_acquire("192.0.2.1"),
            "future panic leaked its connection slot"
        );
        guard.release("192.0.2.1");
    }

    #[actix_web::test]
    async fn completed_success_and_error_responses_release_the_connection_slot() {
        for success in [true, false] {
            let guard = Data::new(IpConnectionGuard::new(1));
            let service = ConnectionLimiter
                .new_transform(actix_web::dev::fn_service(
                    move |req: ServiceRequest| async move {
                        if success {
                            Ok(req.into_response(actix_web::HttpResponse::Ok().finish()))
                        } else {
                            Err(actix_web::error::ErrorBadRequest(
                                "handler rejected request",
                            ))
                        }
                    },
                ))
                .await
                .unwrap();
            let result = service.call(request(&guard)).await;
            assert_eq!(result.is_ok(), success);
            assert!(guard.try_acquire("192.0.2.1"));
            guard.release("192.0.2.1");
        }
    }
}
