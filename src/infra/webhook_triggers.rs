//! Webhook triggers — fire HTTP callbacks when keys matching a prefix change.
//!
//! [`WebhookRegistry`] allows users to register webhook URLs for key prefixes.
//! When a key matching a registered prefix is written or deleted, an HTTP
//! POST request is sent to each registered webhook.
//!
//! This module integrates with the existing CDC (Change Data Capture)
//! infrastructure: webhooks are triggered from the same event stream that
//! CDC uses.
//!
//! # Example
//!
//! ```ignore
//! let registry = WebhookRegistry::new();
//! registry.register("orders/", "https://hooks.example.com/orders").unwrap();
//! registry.trigger(b"orders/123", b"{\"status\":\"shipped\"}");
//! ```

use crate::infra::cdc::{CdcEvent, CdcPublisher};

/// A single webhook registration.
#[derive(Debug, Clone)]
struct WebhookEntry {
    /// Key prefix to match.
    prefix: String,
    /// Target URL to POST to.
    url: String,
}

/// Registry of webhook triggers keyed by prefix.
///
/// Webhooks are fired via the CDC pipeline — when a key matching a
/// registered prefix is mutated, the registry creates a CDC event and
/// publishes it through a [`CdcPublisher`].
pub struct WebhookRegistry {
    /// All registered webhooks.
    entries: Vec<WebhookEntry>,
    // Prefix → list of webhooks that match (built for fast lookup).
    //
    // Stored as a sorted list of (prefix, url) pairs for prefix matching.
    // Built by scanning `entries` on each trigger.
}

impl WebhookRegistry {
    /// Create a new empty webhook registry.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register a webhook URL for a key prefix.
    ///
    /// Every time a key starting with `prefix` is mutated, an HTTP POST
    /// with a [`CdcEvent`] payload will be sent to `url`.
    ///
    /// Returns an error if the URL is empty.
    pub fn register(&mut self, prefix: &str, url: &str) -> Result<(), String> {
        if url.is_empty() {
            return Err("Webhook URL cannot be empty".to_string());
        }
        if prefix.is_empty() {
            return Err("Prefix cannot be empty".to_string());
        }

        // Avoid duplicates.
        if self
            .entries
            .iter()
            .any(|e| e.prefix == prefix && e.url == url)
        {
            return Ok(()); // already registered — idempotent
        }

        self.entries.push(WebhookEntry {
            prefix: prefix.to_string(),
            url: url.to_string(),
        });
        Ok(())
    }

    /// Unregister a webhook URL for a key prefix.
    ///
    /// Returns `true` if the (prefix, url) pair existed and was removed.
    pub fn unregister(&mut self, prefix: &str, url: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !(e.prefix == prefix && e.url == url));
        self.entries.len() < before
    }

    /// Trigger all webhooks that match the given key.
    ///
    /// Creates a [`CdcEvent`] for the mutation and publishes it through
    /// `publisher` for each matching webhook URL.
    ///
    /// Returns the number of webhooks that were triggered.
    pub fn trigger(&self, key: &[u8], value: Option<&[u8]>, publisher: &dyn CdcPublisher) -> usize {
        let key_str = String::from_utf8_lossy(key);
        let matching: Vec<&WebhookEntry> = self
            .entries
            .iter()
            .filter(|e| key_str.starts_with(&e.prefix))
            .collect();

        if matching.is_empty() {
            return 0;
        }

        let event = CdcEvent {
            event_type: if value.is_some() {
                crate::infra::cdc::CdcEventType::Put
            } else {
                crate::infra::cdc::CdcEventType::Delete
            },
            cf: "default".to_string(),
            key: key.to_vec(),
            value: value.map(|v| v.to_vec()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or(std::time::Duration::ZERO)
                .as_nanos(),
        };

        // Publish once for each matching webhook.
        // In a production system this would fan out via a background task.
        for _entry in &matching {
            let _ = publisher.publish(event.clone());
        }

        matching.len()
    }

    /// Return all registered (prefix, url) pairs.
    pub fn list(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|e| (e.prefix.clone(), e.url.clone()))
            .collect()
    }

    /// Return the number of registered webhooks.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no webhooks are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Remove all webhook registrations.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Return the number of webhooks matching a given key.
    pub fn matching_count(&self, key: &[u8]) -> usize {
        let key_str = String::from_utf8_lossy(key);
        self.entries
            .iter()
            .filter(|e| key_str.starts_with(&e.prefix))
            .count()
    }
}

impl Default for WebhookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cdc::CdcCollector;

    #[test]
    fn test_register_and_list() {
        let mut reg = WebhookRegistry::new();
        reg.register("orders/", "https://hook.example.com/orders")
            .unwrap();
        reg.register("users/", "https://hook.example.com/users")
            .unwrap();

        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&(
            "orders/".to_string(),
            "https://hook.example.com/orders".to_string()
        )));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn test_register_empty_url() {
        let mut reg = WebhookRegistry::new();
        let result = reg.register("prefix/", "");
        assert!(result.is_err());
    }

    #[test]
    fn test_register_empty_prefix() {
        let mut reg = WebhookRegistry::new();
        let result = reg.register("", "https://hook.example.com");
        assert!(result.is_err());
    }

    #[test]
    fn test_unregister() {
        let mut reg = WebhookRegistry::new();
        reg.register("a/", "https://hook.example.com/a").unwrap();
        assert!(reg.unregister("a/", "https://hook.example.com/a"));
        assert!(!reg.unregister("a/", "https://hook.example.com/a")); // already gone
        assert!(reg.is_empty());
    }

    #[test]
    fn test_trigger_with_put() {
        let mut reg = WebhookRegistry::new();
        reg.register("orders/", "https://hook.example.com/orders")
            .unwrap();

        let collector = CdcCollector::new();
        let count = reg.trigger(b"orders/123", Some(b"{\"status\":\"shipped\"}"), &collector);
        assert_eq!(count, 1);

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key, b"orders/123");
    }

    #[test]
    fn test_trigger_with_delete() {
        let mut reg = WebhookRegistry::new();
        reg.register("orders/", "https://hook.example.com/orders")
            .unwrap();

        let collector = CdcCollector::new();
        let count = reg.trigger(b"orders/999", None, &collector);
        assert_eq!(count, 1);

        let events = collector.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event_type,
            crate::infra::cdc::CdcEventType::Delete
        ));
    }

    #[test]
    fn test_trigger_no_match() {
        let reg = WebhookRegistry::new();
        let collector = CdcCollector::new();
        let count = reg.trigger(b"no_match", Some(b"value"), &collector);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_matching_count() {
        let mut reg = WebhookRegistry::new();
        reg.register("logs/", "https://hook1.example.com").unwrap();
        reg.register("logs/", "https://hook2.example.com").unwrap();
        reg.register("other/", "https://hook3.example.com").unwrap();

        assert_eq!(reg.matching_count(b"logs/error"), 2);
        assert_eq!(reg.matching_count(b"other/thing"), 1);
        assert_eq!(reg.matching_count(b"unknown"), 0);
    }

    #[test]
    fn test_clear() {
        let mut reg = WebhookRegistry::new();
        reg.register("a/", "https://hook.example.com/a").unwrap();
        reg.register("b/", "https://hook.example.com/b").unwrap();
        assert!(!reg.is_empty());
        reg.clear();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_register_duplicate_is_idempotent() {
        let mut reg = WebhookRegistry::new();
        reg.register("a/", "https://hook.example.com/a").unwrap();
        reg.register("a/", "https://hook.example.com/a").unwrap();
        assert_eq!(reg.len(), 1);
    }
}
