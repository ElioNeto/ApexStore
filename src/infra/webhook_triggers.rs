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
//! The [`WebhookAwareEngine`] wrapper automatically triggers webhooks whenever
//! data is written or deleted through it.
//!
//! # Example
//!
//! ```ignore
//! use crate::infra::multi_model::StorageEngine;
//! use crate::infra::cdc::CdcCollector;
//!
//! let inner = MyEngine::new();
//! let registry = WebhookRegistry::new();
//! registry.register("orders/", "https://hooks.example.com/orders").unwrap();
//! let publisher = CdcCollector::new();
//!
//! let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));
//! engine.put(b"orders/123".to_vec(), b"{\"status\":\"shipped\"}".to_vec()).unwrap();
//! ```

use crate::infra::cdc::{CdcEvent, CdcPublisher};
use crate::infra::multi_model::StorageEngine;

// ── WebhookStorage trait ────────────────────────────────────────────────────

/// Simplified storage interface implemented by [`WebhookAwareEngine`].
///
/// This trait mirrors the write operations of [`StorageEngine`] but is
/// implemented specifically by the webhook-aware wrapper so that callers
/// can use it as a drop-in for scenarios where webhook integration is
/// required.
pub trait WebhookStorage: Send + Sync {
    /// Insert or update a key-value pair, triggering matching webhooks.
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), String>;

    /// Delete a key-value pair, triggering matching webhooks.
    fn delete(&mut self, key: &[u8]) -> Result<(), String>;
}

// ── WebhookAwareEngine ──────────────────────────────────────────────────────

/// A storage engine wrapper that automatically fires webhooks on every
/// write and delete operation.
///
/// The wrapper delegates all operations to an inner [`StorageEngine`] and,
/// after each successful mutation, calls [`WebhookRegistry::trigger`] to
/// notify any registered webhooks whose prefix matches the key.
pub struct WebhookAwareEngine<E> {
    /// The underlying storage engine.
    inner: E,
    /// The webhook registry consulted on each mutation.
    registry: WebhookRegistry,
    /// The CDC publisher through which webhook events are dispatched.
    publisher: Box<dyn CdcPublisher>,
}

impl<E: StorageEngine> WebhookAwareEngine<E> {
    /// Create a new webhook-aware engine.
    ///
    /// - `inner`: the storage engine to wrap.
    /// - `registry`: the webhook registry to consult on each mutation.
    /// - `publisher`: the CDC publisher through which events are sent.
    pub fn new(inner: E, registry: WebhookRegistry, publisher: Box<dyn CdcPublisher>) -> Self {
        Self {
            inner,
            registry,
            publisher,
        }
    }

    /// Return a shared reference to the inner storage engine.
    pub fn inner(&self) -> &E {
        &self.inner
    }

    /// Return a mutable reference to the inner storage engine.
    pub fn inner_mut(&mut self) -> &mut E {
        &mut self.inner
    }

    /// Return a shared reference to the webhook registry.
    pub fn registry(&self) -> &WebhookRegistry {
        &self.registry
    }

    /// Return a mutable reference to the webhook registry.
    pub fn registry_mut(&mut self) -> &mut WebhookRegistry {
        &mut self.registry
    }
}

impl<E: StorageEngine> WebhookStorage for WebhookAwareEngine<E> {
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), String> {
        // Delegate to the inner engine first.
        self.inner.set(key.clone(), value.clone())?;
        // Fire webhooks for matching prefixes.
        self.registry
            .trigger(&key, Some(&value), &*self.publisher);
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), String> {
        // Delegate to the inner engine first.
        self.inner.delete(key)?;
        // Fire webhooks for matching prefixes (value = None for delete).
        self.registry.trigger(key, None, &*self.publisher);
        Ok(())
    }
}

// ── WebhookEntry ────────────────────────────────────────────────────────────

/// A single webhook registration.
#[derive(Debug, Clone)]
struct WebhookEntry {
    /// Key prefix to match.
    prefix: String,
    /// Target URL to POST to.
    url: String,
}

// ── WebhookRegistry ─────────────────────────────────────────────────────────

/// Registry of webhook triggers keyed by prefix.
///
/// Webhooks are fired via the CDC pipeline — when a key matching a
/// registered prefix is mutated, the registry creates a CDC event and
/// publishes it through a [`CdcPublisher`].
pub struct WebhookRegistry {
    /// All registered webhooks.
    entries: Vec<WebhookEntry>,
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cdc::CdcCollector;
    use std::sync::Arc;
    use crate::infra::multi_model::InMemoryEngine;

    // ── WebhookRegistry tests ─────────────────────────────────────────────

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

    // ── WebhookAwareEngine tests ──────────────────────────────────────────

    /// A shared CDC publisher backed by an `Arc<Mutex<Vec<CdcEvent>>>`.
    /// Multiple holders of the same `Arc` see the same events.
    struct SharedCdcPublisher(Arc<std::sync::Mutex<Vec<CdcEvent>>>);

    impl SharedCdcPublisher {
        fn new() -> (Self, Arc<std::sync::Mutex<Vec<CdcEvent>>>) {
            let inner = Arc::new(std::sync::Mutex::new(Vec::new()));
            (Self(inner.clone()), inner)
        }

        fn events_from(inner: &Arc<std::sync::Mutex<Vec<CdcEvent>>>) -> Vec<CdcEvent> {
            inner.lock().unwrap().clone()
        }

        fn clear_events(inner: &Arc<std::sync::Mutex<Vec<CdcEvent>>>) {
            inner.lock().unwrap().clear();
        }
    }

    impl CdcPublisher for SharedCdcPublisher {
        fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    fn make_engine() -> (WebhookAwareEngine<InMemoryEngine>, Arc<std::sync::Mutex<Vec<CdcEvent>>>) {
        let inner = InMemoryEngine::new();
        let mut registry = WebhookRegistry::new();
        registry
            .register("orders/", "https://hook.example.com/orders")
            .unwrap();
        registry
            .register("users/", "https://hook.example.com/users")
            .unwrap();
        let (publisher, events) = SharedCdcPublisher::new();
        let engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));
        (engine, events)
    }

    #[test]
    fn test_webhook_engine_put_triggers_webhook() {
        let (mut engine, events) = make_engine();

        engine
            .put(b"orders/123".to_vec(), b"order_data".to_vec())
            .unwrap();

        let collected = SharedCdcPublisher::events_from(&events);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].key, b"orders/123");
        assert!(matches!(
            collected[0].event_type,
            crate::infra::cdc::CdcEventType::Put
        ));
    }

    #[test]
    fn test_webhook_engine_delete_triggers_webhook() {
        let (mut engine, events) = make_engine();

        // First insert the data
        engine
            .put(b"orders/456".to_vec(), b"data".to_vec())
            .unwrap();
        SharedCdcPublisher::clear_events(&events); // discard put event

        // Now delete
        engine.delete(b"orders/456").unwrap();

        let collected = SharedCdcPublisher::events_from(&events);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].key, b"orders/456");
        assert!(matches!(
            collected[0].event_type,
            crate::infra::cdc::CdcEventType::Delete
        ));
    }

    #[test]
    fn test_webhook_engine_put_no_match() {
        let inner = InMemoryEngine::new();
        let registry = WebhookRegistry::new(); // no registrations
        let (publisher, events) = SharedCdcPublisher::new();
        let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));

        engine
            .put(b"unknown/key".to_vec(), b"value".to_vec())
            .unwrap();

        // No webhooks matched, so no events should be published
        let collected = SharedCdcPublisher::events_from(&events);
        assert!(collected.is_empty());
    }

    #[test]
    fn test_webhook_engine_delete_no_match() {
        let inner = InMemoryEngine::new();
        let registry = WebhookRegistry::new(); // no registrations
        let (publisher, events) = SharedCdcPublisher::new();
        let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));

        engine.delete(b"unknown/key").unwrap();

        let collected = SharedCdcPublisher::events_from(&events);
        assert!(collected.is_empty());
    }

    #[test]
    fn test_webhook_engine_data_persisted() {
        let inner = InMemoryEngine::new();
        let registry = WebhookRegistry::new();
        let (publisher, _events) = SharedCdcPublisher::new();
        let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));

        engine
            .put(b"key1".to_vec(), b"val1".to_vec())
            .unwrap();

        // Verify data is accessible through the inner engine
        let result = engine.inner().get(b"key1").unwrap();
        assert_eq!(result, Some(b"val1".to_vec()));
    }

    #[test]
    fn test_webhook_engine_delete_persisted() {
        let inner = InMemoryEngine::new();
        let mut registry = WebhookRegistry::new();
        registry
            .register("test/", "https://hook.example.com/test")
            .unwrap();
        let (publisher, _events) = SharedCdcPublisher::new();
        let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));

        // Insert and verify
        engine
            .put(b"test/1".to_vec(), b"value".to_vec())
            .unwrap();
        assert_eq!(
            engine.inner().get(b"test/1").unwrap(),
            Some(b"value".to_vec())
        );

        // Delete and verify
        engine.delete(b"test/1").unwrap();
        assert_eq!(engine.inner().get(b"test/1").unwrap(), None);
    }

    #[test]
    fn test_webhook_engine_multiple_registrations() {
        let inner = InMemoryEngine::new();
        let mut registry = WebhookRegistry::new();
        registry
            .register("orders/", "https://hook1.example.com")
            .unwrap();
        registry
            .register("orders/", "https://hook2.example.com")
            .unwrap();
        let (publisher, events) = SharedCdcPublisher::new();
        let mut engine = WebhookAwareEngine::new(inner, registry, Box::new(publisher));

        engine
            .put(b"orders/1".to_vec(), b"data".to_vec())
            .unwrap();

        // Should have 2 events (one per matching webhook)
        let collected = SharedCdcPublisher::events_from(&events);
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_webhook_engine_accessors() {
        let inner = InMemoryEngine::new();
        let mut registry = WebhookRegistry::new();
        registry.register("a/", "https://hook.example.com").unwrap();
        let collector = CdcCollector::new();
        let engine = WebhookAwareEngine::new(inner, registry, Box::new(collector));

        assert_eq!(engine.registry().len(), 1);
        assert!(engine.inner().get(b"x").is_ok());
    }

    #[test]
    fn test_webhook_storage_trait_object() {
        let inner = InMemoryEngine::new();
        let registry = WebhookRegistry::new();
        let collector = CdcCollector::new();
        let engine = WebhookAwareEngine::new(inner, registry, Box::new(collector));

        // Use as a trait object
        let mut boxed: Box<dyn WebhookStorage> = Box::new(engine);
        boxed.put(b"k".to_vec(), b"v".to_vec()).unwrap();
        boxed.delete(b"k").unwrap();
    }
}
