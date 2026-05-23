//! Change Data Capture (CDC) — stream data changes to external systems.
//!
//! This module provides:
//!
//! - [`CdcEvent`] — a data-change event with key, value, timestamp and column family.
//! - [`CdcPublisher`] — a trait for publishing CDC events.
//! - [`CdcConfig`] — configuration for CDC (enabled flag + optional HTTP endpoint).
//! - [`CdcCollector`] — an in-memory collector that records events to a `Vec` (useful for testing).
//! - [`WebhookPublisher`] — a publisher that sends events as HTTP POST to a configured endpoint.

use serde::Serialize;

/// Configuration for Change Data Capture.
#[derive(Debug, Clone, Serialize, Default)]
pub struct CdcConfig {
    /// Whether CDC is enabled.
    pub enabled: bool,
    /// Optional HTTP endpoint to which CDC events are posted (used by [`WebhookPublisher`]).
    pub endpoint: Option<String>,
}

impl CdcConfig {
    /// Create a new disabled CDC config.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Create a new CDC config with an HTTP endpoint.
    pub fn with_endpoint(endpoint: String) -> Self {
        Self {
            enabled: true,
            endpoint: Some(endpoint),
        }
    }
}

/// The type of a CDC event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CdcEventType {
    /// A key-value pair was inserted or updated.
    Put,
    /// A key was deleted.
    Delete,
}

/// A single CDC event representing a data change in the engine.
#[derive(Debug, Clone, Serialize)]
pub struct CdcEvent {
    /// The type of mutation.
    #[serde(rename = "type")]
    pub event_type: CdcEventType,
    /// The column family in which the change occurred.
    pub cf: String,
    /// The key that was mutated.
    #[serde(with = "hex_serde")]
    pub key: Vec<u8>,
    /// The new value (present for `Put`, absent for `Delete`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<u8>>,
    /// Monotonic timestamp in nanoseconds since the Unix epoch.
    pub timestamp: u128,
}

/// Trait for CDC publishers.
///
/// Implementations must be `Send + Sync` so they can be shared across threads
/// (e.g. from within the engine's lock-free sections and actix-web handlers).
pub trait CdcPublisher: Send + Sync {
    /// Publish a single CDC event.
    ///
    /// Returns `Ok(())` on success or an error description on failure.
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// In-memory CDC collector that records events to a `Vec`.
///
/// Useful for testing: after performing engine operations, call [`events`](CdcCollector::events)
/// to inspect the captured mutations.
pub struct CdcCollector {
    events: std::sync::Mutex<Vec<CdcEvent>>,
}

impl CdcCollector {
    /// Create a new empty collector.
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all events recorded so far.
    pub fn events(&self) -> Vec<CdcEvent> {
        self.events.lock().unwrap().clone()
    }

    /// Clear all recorded events.
    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl Clone for CdcCollector {
    fn clone(&self) -> Self {
        Self {
            events: std::sync::Mutex::new(self.events.lock().unwrap().clone()),
        }
    }
}

impl Default for CdcCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl CdcPublisher for CdcCollector {
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

/// A CDC publisher that sends events as HTTP POST requests to a configurable endpoint.
///
/// The event body is serialised as JSON with `Content-Type: application/json`.
/// Uses a short (5 s) connect and read timeout to avoid blocking the engine for long.
pub struct WebhookPublisher {
    endpoint: String,
    agent: ureq::Agent,
}

impl WebhookPublisher {
    /// Create a new webhook publisher targeting `endpoint`.
    ///
    /// The endpoint should be a full URL such as `http://example.com/webhook`.
    pub fn new(endpoint: String) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(5))
            .timeout_read(std::time::Duration::from_secs(5))
            .build();
        Self { endpoint, agent }
    }
}

impl CdcPublisher for WebhookPublisher {
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let json = serde_json::to_string(&event)?;
        self.agent
            .post(&self.endpoint)
            .set("Content-Type", "application/json")
            .send_string(&json)?;
        Ok(())
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    #[allow(dead_code)]
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

// ── Factory helpers ──────────────────────────────────────────────────────────

/// Create a [`CdcPublisher`] box from a [`CdcConfig`].
///
/// * If `config.enabled` is `false`, returns `None`.
/// * If `config.enabled` is `true` and `config.endpoint` is `Some(url)`, returns
///   a [`WebhookPublisher`] targeting that URL.
/// * If `config.enabled` is `true` but `config.endpoint` is `None`, returns
///   a [`CdcCollector`] (in-memory).
pub fn create_publisher(config: &CdcConfig) -> Option<Box<dyn CdcPublisher>> {
    if !config.enabled {
        return None;
    }
    match &config.endpoint {
        Some(url) if !url.is_empty() => Some(Box::new(WebhookPublisher::new(url.clone()))),
        _ => Some(Box::new(CdcCollector::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event() -> CdcEvent {
        CdcEvent {
            event_type: CdcEventType::Put,
            cf: "default".to_string(),
            key: b"test_key".to_vec(),
            value: Some(b"test_value".to_vec()),
            timestamp: 42_000_000_000,
        }
    }

    #[test]
    fn test_cdc_collector_records_events() {
        let collector = CdcCollector::new();
        collector.publish(make_event()).unwrap();
        assert_eq!(collector.events().len(), 1);
        assert!(matches!(
            collector.events()[0].event_type,
            CdcEventType::Put
        ));
    }

    #[test]
    fn test_cdc_collector_clear() {
        let collector = CdcCollector::new();
        collector.publish(make_event()).unwrap();
        collector.clear();
        assert!(collector.events().is_empty());
    }

    #[test]
    fn test_create_publisher_disabled() {
        let config = CdcConfig::disabled();
        assert!(create_publisher(&config).is_none());
    }

    #[test]
    fn test_create_publisher_enabled_no_endpoint() {
        let config = CdcConfig {
            enabled: true,
            endpoint: None,
        };
        let publisher = create_publisher(&config);
        assert!(publisher.is_some());
        // Should create a CdcCollector when no endpoint
        publisher
            .unwrap()
            .publish(make_event())
            .expect("CdcCollector should accept events");
    }

    #[test]
    fn test_cdc_event_serialization() {
        let event = CdcEvent {
            event_type: CdcEventType::Put,
            cf: "default".to_string(),
            key: b"hello".to_vec(),
            value: Some(b"world".to_vec()),
            timestamp: 123,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"put""#));
        assert!(json.contains(r#""cf":"default""#));
        assert!(json.contains(r#""key":"68656c6c6f""#)); // hex of "hello"
        assert!(json.contains(r#""value":"#)); // value should be present (serialized as array since no hex on Option)
    }

    #[test]
    fn test_cdc_event_delete_serialization() {
        let event = CdcEvent {
            event_type: CdcEventType::Delete,
            cf: "test_cf".to_string(),
            key: b"delete_me".to_vec(),
            value: None,
            timestamp: 456,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""type":"delete""#));
        assert!(!json.contains(r#""value""#)); // no value field for delete
    }
}
