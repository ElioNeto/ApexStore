//! Request deduplication and idempotency key support.
//!
//! Stores idempotency keys with cached responses so that duplicate requests
//! (same idempotency key) return the same response without re-executing the
//! operation. Keys have a configurable TTL after which they are cleaned up.
//!
//! This can be wired into the API server as middleware.
//!
//! # Usage
//!
//! ```rust
//! use apexstore::infra::idempotency::IdempotencyMiddleware;
//! use std::time::Duration;
//!
//! let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
//!
//! // Check if a key was already processed
//! if idem.check_idempotency("req-123").is_none() {
//!     // Process request
//!     idem.store_idempotency("req-123", "response_data");
//! }
//!
//! // Later, cleanup expired entries
//! idem.cleanup_expired();
//! ```

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A cached response associated with an idempotency key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    /// The response body as bytes.
    pub body: Vec<u8>,
    /// HTTP status code.
    pub status_code: u16,
    /// Timestamp (Unix epoch millis) when this entry expires.
    pub expires_at: u64,
    /// Timestamp (Unix epoch millis) when this entry was created.
    pub created_at: u64,
}

/// Manages idempotency keys with TTL-based cleanup.
pub struct IdempotencyMiddleware {
    /// In-memory cache of idempotency keys → responses.
    cache: Mutex<HashMap<String, CachedResponse>>,
    /// Default TTL for new entries.
    default_ttl: Duration,
    /// Number of cache hits (for metrics).
    hits: Mutex<u64>,
    /// Number of cache misses.
    misses: Mutex<u64>,
}

impl IdempotencyMiddleware {
    /// Create a new `IdempotencyMiddleware` with the given default TTL.
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            default_ttl,
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// Check if a response for the given idempotency key is cached.
    ///
    /// Returns `Some(CachedResponse)` if the key exists and hasn't expired,
    /// `None` otherwise.
    pub fn check_idempotency(&self, key: &str) -> Option<CachedResponse> {
        let mut cache = self.cache.lock();
        let now_millis = current_time_millis();

        match cache.get(key) {
            Some(entry) if entry.expires_at > now_millis => {
                *self.hits.lock() += 1;
                Some(entry.clone())
            }
            Some(_) => {
                // Expired entry — remove it
                cache.remove(key);
                *self.misses.lock() += 1;
                None
            }
            None => {
                *self.misses.lock() += 1;
                None
            }
        }
    }

    /// Store a response for an idempotency key.
    ///
    /// The entry will expire after the configured TTL.
    pub fn store_idempotency(&self, key: &str, response: &str) {
        let now_millis = current_time_millis();
        let expires_at = now_millis + self.default_ttl.as_millis() as u64;

        let entry = CachedResponse {
            body: response.as_bytes().to_vec(),
            status_code: 200,
            expires_at,
            created_at: now_millis,
        };

        self.cache.lock().insert(key.to_string(), entry);
    }

    /// Store a response with explicit status code.
    pub fn store_idempotency_with_status(
        &self,
        key: &str,
        body: Vec<u8>,
        status_code: u16,
    ) {
        let now_millis = current_time_millis();
        let expires_at = now_millis + self.default_ttl.as_millis() as u64;

        let entry = CachedResponse {
            body,
            status_code,
            expires_at,
            created_at: now_millis,
        };

        self.cache.lock().insert(key.to_string(), entry);
    }

    /// Remove all expired entries from the cache.
    pub fn cleanup_expired(&self) {
        let mut cache = self.cache.lock();
        let now_millis = current_time_millis();
        let before = cache.len();
        cache.retain(|_, entry| entry.expires_at > now_millis);
        let removed = before - cache.len();
        if removed > 0 {
            tracing::debug!("Idempotency: cleaned up {} expired entries", removed);
        }
    }

    /// Remove a specific idempotency key.
    pub fn remove(&self, key: &str) {
        self.cache.lock().remove(key);
    }

    /// Get the number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    /// Returns `true` if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.lock().is_empty()
    }

    /// Get cache hit count.
    pub fn hits(&self) -> u64 {
        *self.hits.lock()
    }

    /// Get cache miss count.
    pub fn misses(&self) -> u64 {
        *self.misses.lock()
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.cache.lock().clear();
    }
}

/// Get current time in milliseconds since Unix epoch.
fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_missing_key() {
        let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
        assert!(idem.check_idempotency("nonexistent").is_none());
        assert_eq!(idem.misses(), 1);
    }

    #[test]
    fn test_store_and_retrieve() {
        let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
        idem.store_idempotency("req-1", "response-1");
        let cached = idem.check_idempotency("req-1");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().status_code, 200);
        assert_eq!(idem.hits(), 1);
    }

    #[test]
    fn test_cleanup_expired() {
        // Use 0 TTL so entries expire immediately
        let idem = IdempotencyMiddleware::new(Duration::from_millis(0));
        idem.store_idempotency("req-expire", "data");
        assert!(idem.check_idempotency("req-expire").is_none());
        assert_eq!(idem.len(), 0); // Should be auto-removed on check
    }

    #[test]
    fn test_remove() {
        let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
        idem.store_idempotency("key-to-remove", "data");
        assert_eq!(idem.len(), 1);
        idem.remove("key-to-remove");
        assert!(idem.is_empty());
    }

    #[test]
    fn test_clear() {
        let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
        idem.store_idempotency("k1", "v1");
        idem.store_idempotency("k2", "v2");
        assert_eq!(idem.len(), 2);
        idem.clear();
        assert!(idem.is_empty());
    }

    #[test]
    fn test_store_with_status() {
        let idem = IdempotencyMiddleware::new(Duration::from_secs(3600));
        idem.store_idempotency_with_status("err-req", b"error".to_vec(), 429);
        let cached = idem.check_idempotency("err-req").unwrap();
        assert_eq!(cached.status_code, 429);
        assert_eq!(cached.body, b"error");
    }
}
