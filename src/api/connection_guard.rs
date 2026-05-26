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
use std::collections::HashMap;
use std::future::{ready, Ready};
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

/// Default maximum number of concurrent requests allowed per IP address.
pub const MAX_CONNECTIONS_PER_IP: usize = 100;

/// Guard that tracks per-IP concurrent connections.
///
/// Call [`try_acquire`](Self::try_acquire) before processing a request and
/// [`release`](Self::release) after the response is sent to the client.
pub struct IpConnectionGuard {
    connections: Mutex<HashMap<String, usize>>,
    /// Maximum number of concurrent requests allowed per IP.
    max_per_ip: usize,
}

impl IpConnectionGuard {
    /// Create a new empty guard with the given per-IP limit.
    pub fn new(max_per_ip: usize) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            max_per_ip,
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
        let mut map = self
            .connections
            .lock()
            .expect("IpConnectionGuard lock poisoned");
        let count = map.entry(ip.to_string()).or_insert(0);
        if *count >= max_per_ip {
            return false;
        }
        *count += 1;
        true
    }

    /// Returns the configured per-IP limit.
    pub fn max_per_ip(&self) -> usize {
        self.max_per_ip
    }

    /// Release a connection slot for `ip`.
    ///
    /// Must be called exactly once for every successful [`try_acquire`](Self::try_acquire).
    pub fn release(&self, ip: &str) {
        let mut map = self
            .connections
            .lock()
            .expect("IpConnectionGuard lock poisoned");
        if let Some(count) = map.get_mut(ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(ip);
            }
        }
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
        // Try to acquire a connection slot for this IP
        let should_reject = if let Some(guard) = req.app_data::<Data<IpConnectionGuard>>() {
            let ip = req
                .peer_addr()
                .map(|s| s.ip().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            if !guard.try_acquire(&ip) {
                true
            } else {
                // We'll release in a wrapper future
                false
            }
        } else {
            false
        };

        if should_reject {
            return Box::pin(ready(Err(actix_web::error::ErrorTooManyRequests(
                "too many concurrent connections from this IP",
            ))));
        }

        let guard = req.app_data::<Data<IpConnectionGuard>>().cloned();
        let ip = req
            .peer_addr()
            .map(|s| s.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let fut = self.service.call(req);
        Box::pin(async move {
            match fut.await {
                Ok(resp) => {
                    if let Some(ref g) = guard {
                        g.release(&ip);
                    }
                    Ok(resp)
                }
                Err(e) => {
                    if let Some(ref g) = guard {
                        g.release(&ip);
                    }
                    Err(e)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let map = guard.connections.lock().unwrap();
        assert!(!map.contains_key("10.0.0.3"));
    }

    #[test]
    fn test_default_max_per_ip() {
        let guard = IpConnectionGuard::new(MAX_CONNECTIONS_PER_IP);
        assert_eq!(guard.max_per_ip(), MAX_CONNECTIONS_PER_IP);
    }
}
