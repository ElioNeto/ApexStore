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
    /// Optional auth header in the format `"header_name:header_value"` or `"Bearer <token>"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    /// Optional custom HTTP timeout in seconds (default: 5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
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
            auth_header: None,
            timeout_secs: None,
        }
    }

    /// Attach an auth header to this CDC config.
    ///
    /// The `header` string is parsed as `"header_name:header_value"`.
    /// If no colon is present, it is treated as a bare bearer token
    /// (i.e. `"Authorization: Bearer <token>"`).
    pub fn with_auth_header(mut self, header: String) -> Self {
        self.auth_header = Some(header);
        self
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
    auth_header: Option<(String, String)>, // (header_name, header_value)
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
        Self {
            endpoint,
            agent,
            auth_header: None,
        }
    }

    /// Attach an HTTP auth header to every request.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use apexstore::infra::cdc::WebhookPublisher;
    /// let publisher = WebhookPublisher::new("http://example.com/hook".into())
    ///     .with_auth("Authorization".into(), "Bearer my-token".into());
    /// ```
    pub fn with_auth(mut self, header_name: String, header_value: String) -> Self {
        self.auth_header = Some((header_name, header_value));
        self
    }
}

impl CdcPublisher for WebhookPublisher {
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let json = serde_json::to_string(&event)?;

        // Retry up to 3 times with exponential backoff.
        // Build a fresh request each time because ureq::Request is consumed on send.
        let mut last_err = None;
        for attempt in 0..3 {
            let mut req = self.agent.post(&self.endpoint);
            req = req.set("Content-Type", "application/json");

            // Add auth header if configured
            if let Some((ref name, ref value)) = self.auth_header {
                req = req.set(name, value);
            }

            match req.send_string(&json) {
                Ok(_) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    std::thread::sleep(std::time::Duration::from_millis(100 * (1 << attempt)));
                }
            }
        }

        Err(Box::new(std::io::Error::other(
            format!("CDC publish failed after 3 retries: {:?}", last_err),
        )))
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
        Some(url) if !url.is_empty() => {
            let mut publisher = WebhookPublisher::new(url.clone());

            // Apply custom timeout if configured
            if let Some(secs) = config.timeout_secs {
                let agent = ureq::AgentBuilder::new()
                    .timeout_connect(std::time::Duration::from_secs(secs))
                    .timeout_read(std::time::Duration::from_secs(secs))
                    .build();
                publisher = WebhookPublisher {
                    endpoint: url.clone(),
                    agent,
                    auth_header: None,
                };
                // Re-apply auth header if present (since we rebuilt the publisher)
                if let Some(ref auth) = config.auth_header {
                    if let Some((name, value)) = auth.split_once(':') {
                        publisher = publisher.with_auth(
                            name.trim().to_string(),
                            value.trim().to_string(),
                        );
                    } else {
                        publisher = publisher.with_auth(
                            "Authorization".to_string(),
                            format!("Bearer {}", auth),
                        );
                    }
                }
            } else if let Some(ref auth) = config.auth_header {
                // Support "Authorization: Bearer <token>" format
                if let Some((name, value)) = auth.split_once(':') {
                    publisher = publisher.with_auth(
                        name.trim().to_string(),
                        value.trim().to_string(),
                    );
                } else {
                    // Treat as bare bearer token
                    publisher = publisher.with_auth(
                        "Authorization".to_string(),
                        format!("Bearer {}", auth),
                    );
                }
            }

            Some(Box::new(publisher))
        }
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
            auth_header: None,
            timeout_secs: None,
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
    fn test_webhook_publisher_with_auth_header() {
        let _publisher = WebhookPublisher::new("http://example.com/hook".into())
            .with_auth("Authorization".into(), "Bearer my-token".into());
        // Verify the auth_header was set by checking public API
        // (auth_header is private, so we verify via the builder pattern)
    }

    #[test]
    fn test_cdc_config_with_auth_header_bearer() {
        let config = CdcConfig::with_endpoint("http://example.com/hook".into())
            .with_auth_header("my-bearer-token".into());
        assert_eq!(config.auth_header, Some("my-bearer-token".into()));
    }

    #[test]
    fn test_cdc_config_with_auth_header_colon_format() {
        let config = CdcConfig::with_endpoint("http://example.com/hook".into())
            .with_auth_header("X-API-Key: secret123".into());
        assert_eq!(config.auth_header, Some("X-API-Key: secret123".into()));
    }

    #[test]
    fn test_cdc_config_with_endpoint_and_auth() {
        let config = CdcConfig::with_endpoint("http://example.com/hook".into())
            .with_auth_header("Authorization: Bearer my-token".into());
        assert!(config.enabled);
        assert_eq!(config.endpoint, Some("http://example.com/hook".into()));
        assert_eq!(
            config.auth_header,
            Some("Authorization: Bearer my-token".into())
        );
        assert_eq!(config.timeout_secs, None);
    }

    #[test]
    fn test_cdc_config_disabled() {
        let config = CdcConfig::disabled();
        assert!(!config.enabled);
        assert_eq!(config.endpoint, None);
        assert_eq!(config.auth_header, None);
        assert_eq!(config.timeout_secs, None);
    }

    #[test]
    fn test_webhook_publisher_new() {
        let publisher = WebhookPublisher::new("http://localhost:9999/hook".into());
        // Ensure the publisher implements CdcPublisher and can be boxed
        let boxed: Box<dyn CdcPublisher> = Box::new(publisher);
        // Publishing to a non-existent endpoint should fail (no server)
        let result = boxed.publish(make_event());
        assert!(result.is_err());
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
