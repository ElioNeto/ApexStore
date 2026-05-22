//! Built-in pub/sub messaging over topics.
//!
//! Provides a [`PubSub`] struct that implements a topic-based publish–subscribe
//! pattern using `tokio::sync::broadcast` channels internally.
//!
//! # Example
//!
//! ```ignore
//! let ps = PubSub::new(64);
//! let mut rx = ps.subscribe("events");
//! ps.publish("events", "hello").unwrap();
//! assert_eq!(rx.recv().await.unwrap(), "hello");
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

/// A channel for a single topic.
struct TopicChannel {
    /// Sender half — all publishers share this.
    tx: broadcast::Sender<Vec<u8>>,
}

/// Topic-based publish–subscribe system.
///
/// Internally each topic has a `tokio::sync::broadcast` channel.  Messages
/// are delivered to all active subscribers.  Subscribers that are too slow
/// will be lagged and disconnected (broadcast channel behaviour).
///
/// Messages are raw byte vectors — serialisation is left to the caller.
pub struct PubSub {
    /// Map of topic name → channel.
    topics: Arc<parking_lot::Mutex<HashMap<String, TopicChannel>>>,
    /// Default capacity for new topics (number of messages buffered).
    default_capacity: usize,
}

impl PubSub {
    /// Create a new empty PubSub instance.
    ///
    /// `default_capacity` controls the buffer size for newly created topics.
    pub fn new(default_capacity: usize) -> Self {
        Self {
            topics: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            default_capacity,
        }
    }

    /// Publish a message to a topic.
    ///
    /// All current subscribers of that topic will receive the message.
    /// Returns the number of active subscribers, or `None` if the topic
    /// does not exist.
    pub fn publish(&self, topic: &str, message: Vec<u8>) -> Option<usize> {
        let topics = self.topics.lock();
        topics.get(topic).map(|ch| {
            // Ignore the "no receivers" error — it's not a failure for us.
            let _ = ch.tx.send(message);
            ch.tx.receiver_count()
        })
    }

    /// Publish a string message to a topic (convenience wrapper).
    pub fn publish_str(&self, topic: &str, message: &str) -> Option<usize> {
        self.publish(topic, message.as_bytes().to_vec())
    }

    /// Subscribe to a topic.
    ///
    /// If the topic does not exist yet, it is created with the default capacity.
    /// Returns a `broadcast::Receiver` that will receive all future messages
    /// on that topic.
    pub fn subscribe(&self, topic: &str) -> broadcast::Receiver<Vec<u8>> {
        let mut topics = self.topics.lock();
        let entry = topics.entry(topic.to_string());
        let tx = entry.or_insert_with(|| {
            let (tx, _) = broadcast::channel(self.default_capacity);
            TopicChannel { tx }
        });
        tx.tx.subscribe()
    }

    /// Unsubscribe the given receiver from a topic.
    ///
    /// This simply drops the receiver.  After calling this, the receiver
    /// should not be used anymore.  Returns `true` if the topic still exists
    /// after unsubscription.
    pub fn unsubscribe(&self, topic: &str) -> bool {
        let topics = self.topics.lock();
        topics.contains_key(topic)
    }

    /// Remove a topic entirely, disconnecting all subscribers.
    ///
    /// Returns `true` if the topic existed and was removed.
    pub fn remove_topic(&self, topic: &str) -> bool {
        // Removing the sender causes receivers to get RecvError::Closed.
        let mut topics = self.topics.lock();
        topics.remove(topic).is_some()
    }

    /// Return a list of all active topic names.
    pub fn topics(&self) -> Vec<String> {
        let topics = self.topics.lock();
        topics.keys().cloned().collect()
    }

    /// Return the number of subscribers on a topic.
    pub fn subscriber_count(&self, topic: &str) -> Option<usize> {
        let topics = self.topics.lock();
        topics.get(topic).map(|ch| ch.tx.receiver_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_publish_subscribe() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ps = PubSub::new(16);

            let mut rx = ps.subscribe("events");
            ps.publish_str("events", "hello").unwrap();

            let msg = rx.recv().await.unwrap();
            assert_eq!(msg, b"hello");
        });
    }

    #[test]
    fn test_multiple_subscribers() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ps = PubSub::new(16);

            let mut rx1 = ps.subscribe("alerts");
            let mut rx2 = ps.subscribe("alerts");

            ps.publish_str("alerts", "fire").unwrap();

            let msg1 = rx1.recv().await.unwrap();
            let msg2 = rx2.recv().await.unwrap();
            assert_eq!(msg1, b"fire");
            assert_eq!(msg2, b"fire");
        });
    }

    #[test]
    fn test_publish_to_nonexistent_topic() {
        let ps = PubSub::new(16);
        assert!(ps.publish_str("nowhere", "test").is_none());
    }

    #[test]
    fn test_remove_topic() {
        let ps = PubSub::new(16);
        ps.subscribe("temp");
        assert!(ps.remove_topic("temp"));
        assert!(!ps.remove_topic("temp"));
    }

    #[test]
    fn test_topics_list() {
        let ps = PubSub::new(16);
        ps.subscribe("a");
        ps.subscribe("b");
        let topics = ps.topics();
        assert!(topics.contains(&"a".to_string()));
        assert!(topics.contains(&"b".to_string()));
    }

    #[test]
    fn test_subscriber_count() {
        let ps = PubSub::new(16);
        assert_eq!(ps.subscriber_count("test"), None);

        ps.subscribe("test");
        assert_eq!(ps.subscriber_count("test"), Some(1));

        ps.subscribe("test");
        assert_eq!(ps.subscriber_count("test"), Some(2));
    }

    #[test]
    fn test_unsubscribe() {
        let ps = PubSub::new(16);
        ps.subscribe("topic");
        assert!(ps.unsubscribe("topic"));
    }
}
