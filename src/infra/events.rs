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
