//! Multi-model queries — unified query interface over key-value, vector, time-series,
//! and graph data models.
//!
//! The [`MultiModelEngine`] wraps the core LSM engine along with auxiliary indexes
//! (vector, document, time-series, graph) and dispatches queries to the appropriate
//! subsystem.

use crate::infra::data_tiering::Tier;
use std::collections::HashMap;

/// A generic document value (JSON-like).
pub type Document = HashMap<String, String>;

/// A time-series data point.
#[derive(Debug, Clone)]
pub struct TimeSeriesPoint {
    /// Timestamp (nanoseconds since Unix epoch).
    pub timestamp: u128,
    /// Value at this timestamp.
    pub value: f64,
    /// Optional label/tag.
    pub label: Option<String>,
}

/// A graph vertex.
#[derive(Debug, Clone)]
pub struct GraphVertex {
    /// Unique vertex ID.
    pub id: String,
    /// Vertex label / type.
    pub label: String,
    /// Adjacent vertex IDs.
    pub edges: Vec<String>,
    /// Arbitrary properties.
    pub properties: HashMap<String, String>,
}

/// Multi-model query engine that dispatches queries to the appropriate
/// data model handler.
///
/// # Stub
///
/// This is a skeleton.  A production implementation would delegate to:
///
/// - **Document queries** → the LSM engine (key-value store).
/// - **Time-series queries** → a time-series compaction / retention engine.
/// - **Graph queries** → an adjacency-list index built on top of the LSM engine.
pub struct MultiModelEngine {
    /// Whether document query support is enabled.
    document_enabled: bool,
    /// Whether time-series query support is enabled.
    time_series_enabled: bool,
    /// Whether graph query support is enabled.
    graph_enabled: bool,
}

impl MultiModelEngine {
    /// Create a new multi-model engine.  By default all models are enabled.
    pub fn new() -> Self {
        Self {
            document_enabled: true,
            time_series_enabled: true,
            graph_enabled: true,
        }
    }

    /// Create a new multi-model engine with selective model enablement.
    pub fn with_models(document: bool, time_series: bool, graph: bool) -> Self {
        Self {
            document_enabled: document,
            time_series_enabled: time_series,
            graph_enabled: graph,
        }
    }

    /// Query a document by key.
    ///
    /// Returns the parsed document or an error if document queries are disabled.
    ///
    /// # Stub
    ///
    /// Currently returns a placeholder document.
    pub fn query_document(&self, key: &str) -> Result<Document, String> {
        if !self.document_enabled {
            return Err("Document queries are disabled".to_string());
        }
        let mut doc = HashMap::new();
        doc.insert("key".to_string(), key.to_string());
        doc.insert("value".to_string(), format!("<stub: document for '{}'>", key));
        Ok(doc)
    }

    /// Query time-series data within a time range.
    ///
    /// # Stub
    ///
    /// Currently returns an empty vector.
    pub fn query_time_series(&self, start_ts: u128, end_ts: u128) -> Result<Vec<TimeSeriesPoint>, String> {
        if !self.time_series_enabled {
            return Err("Time-series queries are disabled".to_string());
        }
        let _ = (start_ts, end_ts);
        Ok(Vec::new())
    }

    /// Query a graph vertex by ID.
    ///
    /// Returns the vertex and its adjacency list, or an error if graph
    /// queries are disabled.
    ///
    /// # Stub
    ///
    /// Currently returns a placeholder vertex.
    pub fn query_graph(&self, vertex_id: &str) -> Result<GraphVertex, String> {
        if !self.graph_enabled {
            return Err("Graph queries are disabled".to_string());
        }
        Ok(GraphVertex {
            id: vertex_id.to_string(),
            label: "stub".to_string(),
            edges: Vec::new(),
            properties: HashMap::new(),
        })
    }

    // ── Model toggles ─────────────────────────────────────────────────────────

    /// Enable or disable document queries.
    pub fn set_document_enabled(&mut self, enabled: bool) {
        self.document_enabled = enabled;
    }

    /// Enable or disable time-series queries.
    pub fn set_time_series_enabled(&mut self, enabled: bool) {
        self.time_series_enabled = enabled;
    }

    /// Enable or disable graph queries.
    pub fn set_graph_enabled(&mut self, enabled: bool) {
        self.graph_enabled = enabled;
    }

    /// Returns `true` if document queries are enabled.
    pub fn is_document_enabled(&self) -> bool {
        self.document_enabled
    }

    /// Returns `true` if time-series queries are enabled.
    pub fn is_time_series_enabled(&self) -> bool {
        self.time_series_enabled
    }

    /// Returns `true` if graph queries are enabled.
    pub fn is_graph_enabled(&self) -> bool {
        self.graph_enabled
    }
}

impl Default for MultiModelEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// A tiered data model that embeds the tier of a key alongside its value.
///
/// This type is used by the multi-model engine to return tier-aware results.
pub struct TieredValue {
    /// The key.
    pub key: Vec<u8>,
    /// The raw value.
    pub value: Vec<u8>,
    /// The storage tier of the key.
    pub tier: Tier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_document() {
        let engine = MultiModelEngine::new();
        let doc = engine.query_document("my_key").unwrap();
        assert_eq!(doc.get("key").unwrap(), "my_key");
    }

    #[test]
    fn test_query_document_disabled() {
        let engine = MultiModelEngine::with_models(false, true, true);
        let result = engine.query_document("key");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[test]
    fn test_query_time_series() {
        let engine = MultiModelEngine::new();
        let points = engine.query_time_series(0, 100).unwrap();
        assert!(points.is_empty());
    }

    #[test]
    fn test_query_graph() {
        let engine = MultiModelEngine::new();
        let vertex = engine.query_graph("v1").unwrap();
        assert_eq!(vertex.id, "v1");
    }

    #[test]
    fn test_toggle_models() {
        let mut engine = MultiModelEngine::new();
        assert!(engine.is_document_enabled());
        engine.set_document_enabled(false);
        assert!(!engine.is_document_enabled());
    }
}
