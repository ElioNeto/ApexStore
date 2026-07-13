//! Event bus infrastructure — CDC event broadcasting to subscribers.
//!
//! Provides:
//! - [`EventBus`] — in-process event broadcaster via tokio broadcast channels
//! - [`EventQuery`] — filter for CDC events by key prefix and event type

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::broadcast;

use crate::infra::cdc::{CdcEvent, CdcEventType, CdcPublisher};

/// Capacity of the broadcast channel for CDC events.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// A CDC event publisher that broadcasts events to all connected clients
/// via a tokio broadcast channel.
#[derive(Clone)]
pub struct EventBus {
    /// Broadcast sender for CDC events.
    sender: broadcast::Sender<CdcEvent>,
    /// Whether the event bus is enabled.
    enabled: Arc<AtomicBool>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            sender,
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Return a receiver that can subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<CdcEvent> {
        self.sender.subscribe()
    }

    /// Enable or disable the event bus.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Check if the event bus is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
}

impl CdcPublisher for EventBus {
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !self.enabled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.sender.send(event);
        Ok(())
    }
}

impl CdcPublisher for Arc<EventBus> {
    fn publish(&self, event: CdcEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        (**self).publish(event)
    }
}

/// Filter parameters for CDC event subscriptions.
///
/// Supports optional filtering by key prefix and event type ("put" | "delete").
#[derive(Deserialize)]
pub struct EventQuery {
    /// Optional prefix filter — only receive events for keys matching this prefix.
    pub prefix: Option<String>,
    /// Optional event type filter: "put" or "delete".
    #[serde(rename = "type")]
    pub event_type: Option<String>,
}

impl EventQuery {
    /// Check whether an event passes the filter criteria.
    pub fn matches(&self, event: &CdcEvent) -> bool {
        // Filter by key prefix
        if let Some(ref prefix) = self.prefix {
            let key_str = String::from_utf8_lossy(&event.key);
            if !key_str.starts_with(prefix) {
                return false;
            }
        }
        // Filter by event type
        if let Some(ref evt_type) = self.event_type {
            let matches_type = match evt_type.to_lowercase().as_str() {
                "put" => matches!(event.event_type, CdcEventType::Put),
                "delete" => matches!(event.event_type, CdcEventType::Delete),
                _ => true,
            };
            if !matches_type {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::cdc::{CdcEventType, CdcPublisher};

    fn make_event(key: &str, event_type: CdcEventType) -> CdcEvent {
        CdcEvent {
            event_type,
            cf: "default".to_string(),
            key: key.as_bytes().to_vec(),
            value: Some(b"val".to_vec()),
            timestamp: 0,
        }
    }

    #[test]
    fn test_event_bus_enable_disable() {
        let bus = EventBus::new();
        assert!(!bus.is_enabled());
        bus.set_enabled(true);
        assert!(bus.is_enabled());
        bus.set_enabled(false);
        assert!(!bus.is_enabled());
    }

    #[test]
    fn test_event_bus_subscribe_and_publish() {
        let bus = EventBus::new();
        bus.set_enabled(true);
        let mut rx = bus.subscribe();

        let event = make_event("test_key", CdcEventType::Put);
        bus.publish(event).unwrap();

        // Should receive the published event
        let received = rx.try_recv().unwrap();
        assert_eq!(received.key, b"test_key");
        assert_eq!(received.event_type, CdcEventType::Put);
    }

    #[test]
    fn test_event_bus_disabled_does_not_panic() {
        let bus = EventBus::new();
        // Not enabled — publish should succeed without sending
        let event = make_event("ignored", CdcEventType::Delete);
        assert!(bus.publish(event).is_ok());
    }

    #[test]
    fn test_event_query_prefix_filter() {
        let query = EventQuery {
            prefix: Some("users:".to_string()),
            event_type: None,
        };

        let matching = make_event("users:123", CdcEventType::Put);
        let non_matching = make_event("posts:456", CdcEventType::Put);

        assert!(query.matches(&matching));
        assert!(!query.matches(&non_matching));
    }

    #[test]
    fn test_event_query_type_filter() {
        let query = EventQuery {
            prefix: None,
            event_type: Some("put".to_string()),
        };

        let put_event = make_event("k", CdcEventType::Put);
        let del_event = make_event("k", CdcEventType::Delete);

        assert!(query.matches(&put_event));
        assert!(!query.matches(&del_event));
    }

    #[test]
    fn test_event_query_combined_filter() {
        let query = EventQuery {
            prefix: Some("a:".to_string()),
            event_type: Some("delete".to_string()),
        };

        assert!(query.matches(&make_event("a:1", CdcEventType::Delete)));
        assert!(!query.matches(&make_event("b:1", CdcEventType::Delete)));
        assert!(!query.matches(&make_event("a:1", CdcEventType::Put)));
    }

    #[test]
    fn test_event_query_no_filter_passes_all() {
        let query = EventQuery {
            prefix: None,
            event_type: None,
        };
        assert!(query.matches(&make_event("anything", CdcEventType::Put)));
        assert!(query.matches(&make_event("anything", CdcEventType::Delete)));
    }
}
