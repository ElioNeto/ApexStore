//! Multi-model queries — unified query interface over key-value, vector, time-series,
//! and graph data models.
//!
//! The [`MultiModelEngine`] wraps an LSM storage engine and dispatches queries to
//! the appropriate data model handler.
//!
//! # Storage Engine Trait
//!
//! The [`StorageEngine`] trait abstracts over any key-value store.  The
//! [`InMemoryEngine`] provides a HashMap-backed implementation useful for testing.
//! To use a real LSM engine, implement the trait for your engine type.

use crate::infra::models::data_tiering::Tier;
use std::collections::HashMap;
use std::sync::Mutex;

// ── StorageEngine trait ─────────────────────────────────────────────────────

/// Abstract key-value storage engine that the multi-model layer queries through.
///
/// Implementations must be `Send + Sync` so they can be shared across threads
/// (e.g. from within actix-web handlers).
pub trait StorageEngine: Send + Sync {
    /// Retrieve the value for a key, or `None` if the key does not exist.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String>;

    /// Insert or update a key-value pair.
    fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), String>;

    /// Delete a key-value pair.
    fn delete(&mut self, key: &[u8]) -> Result<(), String>;

    /// Scan all key-value pairs whose key starts with the given prefix,
    /// returned in lexicographic order.
    #[allow(clippy::type_complexity)]
    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String>;
}

// ── In-memory engine (fallback / test) ──────────────────────────────────────

/// A simple in-memory key-value engine backed by a `HashMap` behind a `Mutex`.
///
/// Useful as the default fallback for [`MultiModelEngine`] and for unit tests.
#[derive(Clone)]
pub struct InMemoryEngine {
    data: std::sync::Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
}

impl InMemoryEngine {
    /// Create a new empty in-memory engine.
    pub fn new() -> Self {
        Self {
            data: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageEngine for InMemoryEngine {
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let map = self.data.lock().map_err(|e| e.to_string())?;
        Ok(map.get(key).cloned())
    }

    fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), String> {
        let mut map = self.data.lock().map_err(|e| e.to_string())?;
        map.insert(key, value);
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> Result<(), String> {
        let mut map = self.data.lock().map_err(|e| e.to_string())?;
        map.remove(key);
        Ok(())
    }

    fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        let map = self.data.lock().map_err(|e| e.to_string())?;
        let mut results: Vec<_> = map
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }
}

// ── Data types ──────────────────────────────────────────────────────────────

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

// ── MultiModelEngine ────────────────────────────────────────────────────────

/// Multi-model query engine that dispatches queries to the appropriate
/// data model handler.
///
/// Wraps a [`StorageEngine`] and provides high-level query methods for documents,
/// time-series, and graph data models.  Model support can be toggled individually.
pub struct MultiModelEngine {
    /// The underlying storage engine.
    engine: Box<dyn StorageEngine>,
    /// Whether document query support is enabled.
    document_enabled: bool,
    /// Whether time-series query support is enabled.
    time_series_enabled: bool,
    /// Whether graph query support is enabled.
    graph_enabled: bool,
}

impl MultiModelEngine {
    /// Create a new multi-model engine with an in-memory fallback.
    /// By default all models are enabled.
    pub fn new() -> Self {
        Self {
            engine: Box::new(InMemoryEngine::new()),
            document_enabled: true,
            time_series_enabled: true,
            graph_enabled: true,
        }
    }

    /// Create a new multi-model engine wrapping a custom storage engine.
    /// By default all models are enabled.
    pub fn with_engine(engine: Box<dyn StorageEngine>) -> Self {
        Self {
            engine,
            document_enabled: true,
            time_series_enabled: true,
            graph_enabled: true,
        }
    }

    /// Create a new multi-model engine with selective model enablement,
    /// wrapping an in-memory fallback.
    pub fn with_models(document: bool, time_series: bool, graph: bool) -> Self {
        Self {
            engine: Box::new(InMemoryEngine::new()),
            document_enabled: document,
            time_series_enabled: time_series,
            graph_enabled: graph,
        }
    }

    /// Return a shared reference to the underlying storage engine.
    pub fn engine(&self) -> &dyn StorageEngine {
        &*self.engine
    }

    /// Return a mutable reference to the underlying storage engine.
    pub fn engine_mut(&mut self) -> &mut dyn StorageEngine {
        &mut *self.engine
    }

    // ── Document queries ──────────────────────────────────────────────────

    /// Query a document by key.
    ///
    /// The value stored under `key` is expected to be a sequence of newline-
    /// separated `k=v` pairs.  Returns the parsed document or an error if
    /// document queries are disabled.
    pub fn query_document(&self, key: &str) -> Result<Document, String> {
        if !self.document_enabled {
            return Err("Document queries are disabled".to_string());
        }
        let value = self.engine.get(key.as_bytes())?;
        match value {
            Some(raw) => parse_document(&raw),
            None => Ok(HashMap::new()),
        }
    }

    // ── Time-series queries ───────────────────────────────────────────────

    /// Query time-series data within a time range.
    ///
    /// Keys with the prefix `ts/` are scanned; the remainder of the key is
    /// parsed as a hex-encoded timestamp (32 hex characters, zero-padded u128).
    /// Values are parsed as `{f64_value}|{optional_label}`.
    ///
    /// Only points whose timestamp falls within `(start_ts, end_ts]` are
    /// returned.
    pub fn query_time_series(
        &self,
        start_ts: u128,
        end_ts: u128,
    ) -> Result<Vec<TimeSeriesPoint>, String> {
        if !self.time_series_enabled {
            return Err("Time-series queries are disabled".to_string());
        }
        let entries = self.engine.scan_prefix(b"ts/")?;
        let mut points = Vec::new();
        for (key, value) in entries {
            let key_str = String::from_utf8_lossy(&key);
            // key format: "ts/{timestamp_hex}"
            if let Some(ts_hex) = key_str.strip_prefix("ts/") {
                if let Ok(timestamp) = u128::from_str_radix(ts_hex, 16) {
                    if timestamp > start_ts && timestamp <= end_ts {
                        let value_str = String::from_utf8_lossy(&value);
                        let (val_str, label) = match value_str.split_once('|') {
                            Some((v, l)) => (v, Some(l.to_string())),
                            None => (value_str.as_ref(), None),
                        };
                        if let Ok(val) = val_str.parse::<f64>() {
                            points.push(TimeSeriesPoint {
                                timestamp,
                                value: val,
                                label,
                            });
                        }
                    }
                }
            }
        }
        // Sort by timestamp ascending
        points.sort_by_key(|a| a.timestamp);
        Ok(points)
    }

    // ── Graph queries ─────────────────────────────────────────────────────

    /// Query a graph vertex by ID.
    ///
    /// The vertex data is stored under the key `graph/{vertex_id}` with the
    /// following format:
    ///
    /// ```text
    /// label
    /// edge1,edge2,...
    /// prop1=val1
    /// prop2=val2
    /// ```
    pub fn query_graph(&self, vertex_id: &str) -> Result<GraphVertex, String> {
        if !self.graph_enabled {
            return Err("Graph queries are disabled".to_string());
        }
        let key = format!("graph/{}", vertex_id);
        let value = self.engine.get(key.as_bytes())?;
        match value {
            Some(raw) => parse_graph_vertex(vertex_id, &raw),
            None => Ok(GraphVertex {
                id: vertex_id.to_string(),
                label: String::new(),
                edges: Vec::new(),
                properties: HashMap::new(),
            }),
        }
    }

    // ── Model toggles ─────────────────────────────────────────────────────

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

// ── Parsing helpers ─────────────────────────────────────────────────────────

/// Parse a document value (newline-separated `k=v` pairs).
fn parse_document(raw: &[u8]) -> Result<Document, String> {
    let body = String::from_utf8_lossy(raw);
    let mut doc = HashMap::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            doc.insert(k.to_string(), v.to_string());
        } else {
            return Err(format!("Malformed document line (missing '='): {}", line));
        }
    }
    Ok(doc)
}

/// Parse a graph vertex value.
///
/// Format:
/// ```text
/// label
/// edge1,edge2,...
/// prop1=val1
/// prop2=val2
/// ```
fn parse_graph_vertex(id: &str, raw: &[u8]) -> Result<GraphVertex, String> {
    let body = String::from_utf8_lossy(raw);
    let lines: Vec<&str> = body.lines().collect();
    if lines.is_empty() {
        return Ok(GraphVertex {
            id: id.to_string(),
            label: String::new(),
            edges: Vec::new(),
            properties: HashMap::new(),
        });
    }

    let label = lines[0].to_string();
    let mut edges = Vec::new();
    let mut properties = HashMap::new();

    if lines.len() > 1 {
        // Second line: comma-separated edges (may be empty)
        let edge_str = lines[1].trim();
        if !edge_str.is_empty() {
            edges = edge_str.split(',').map(|s| s.trim().to_string()).collect();
        }
    }

    // Remaining lines: key=value properties
    for line in lines.get(2..).unwrap_or(&[]) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            properties.insert(k.to_string(), v.to_string());
        }
    }

    Ok(GraphVertex {
        id: id.to_string(),
        label,
        edges,
        properties,
    })
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

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── StorageEngine tests ───────────────────────────────────────────────

    #[test]
    fn test_in_memory_engine_get_set_delete() {
        let mut eng = InMemoryEngine::new();
        assert_eq!(eng.get(b"foo").unwrap(), None);

        eng.set(b"foo".to_vec(), b"bar".to_vec()).unwrap();
        assert_eq!(eng.get(b"foo").unwrap(), Some(b"bar".to_vec()));

        eng.delete(b"foo").unwrap();
        assert_eq!(eng.get(b"foo").unwrap(), None);
    }

    #[test]
    fn test_in_memory_engine_scan_prefix() {
        let mut eng = InMemoryEngine::new();
        eng.set(b"ts/100".to_vec(), b"1.0".to_vec()).unwrap();
        eng.set(b"ts/200".to_vec(), b"2.0".to_vec()).unwrap();
        eng.set(b"graph/v1".to_vec(), b"label".to_vec()).unwrap();

        let ts_results = eng.scan_prefix(b"ts/").unwrap();
        assert_eq!(ts_results.len(), 2);

        let graph_results = eng.scan_prefix(b"graph/").unwrap();
        assert_eq!(graph_results.len(), 1);

        let no_results = eng.scan_prefix(b"nonexistent/").unwrap();
        assert!(no_results.is_empty());
    }

    // ── MultiModelEngine tests ────────────────────────────────────────────

    #[test]
    fn test_query_document() {
        let mut engine = MultiModelEngine::new();
        // Set a document value: "k1=v1\nk2=v2"
        let doc_value = b"name=Alice\nrole=engineer";
        engine
            .engine_mut()
            .set(b"user:1".to_vec(), doc_value.to_vec())
            .unwrap();

        let doc = engine.query_document("user:1").unwrap();
        assert_eq!(doc.get("name").unwrap(), "Alice");
        assert_eq!(doc.get("role").unwrap(), "engineer");
    }

    #[test]
    fn test_query_document_not_found() {
        let engine = MultiModelEngine::new();
        let doc = engine.query_document("nonexistent").unwrap();
        assert!(doc.is_empty());
    }

    #[test]
    fn test_query_document_disabled() {
        let engine = MultiModelEngine::with_models(false, true, true);
        let result = engine.query_document("key");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[test]
    fn test_query_document_malformed_line() {
        let mut engine = MultiModelEngine::new();
        // Missing '=' separator
        engine
            .engine_mut()
            .set(b"bad".to_vec(), b"no_equal_sign".to_vec())
            .unwrap();
        let result = engine.query_document("bad");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Malformed"));
    }

    #[test]
    fn test_query_time_series() {
        let mut engine = MultiModelEngine::new();

        // Insert time-series data
        // Key format: ts/{timestamp_hex}
        // Value format: {f64}|{label}
        let ts1: u128 = 1000;
        let ts2: u128 = 2000;
        let ts3: u128 = 3000;

        let key1 = format!("ts/{:032x}", ts1);
        let key2 = format!("ts/{:032x}", ts2);
        let key3 = format!("ts/{:032x}", ts3);

        engine
            .engine_mut()
            .set(key1.into_bytes(), b"1.5|cpu".to_vec())
            .unwrap();
        engine
            .engine_mut()
            .set(key2.into_bytes(), b"2.5|mem".to_vec())
            .unwrap();
        engine
            .engine_mut()
            .set(key3.into_bytes(), b"3.5".to_vec())
            .unwrap();

        // Query range (0, 2500] — should return ts1 and ts2
        let points = engine.query_time_series(0, 2500).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].timestamp, ts1);
        assert_eq!(points[0].value, 1.5);
        assert_eq!(points[0].label.as_deref(), Some("cpu"));
        assert_eq!(points[1].timestamp, ts2);
        assert_eq!(points[1].value, 2.5);
        assert_eq!(points[1].label.as_deref(), Some("mem"));

        // Query with no points in range
        let points = engine.query_time_series(5000, 6000).unwrap();
        assert!(points.is_empty());
    }

    #[test]
    fn test_query_time_series_disabled() {
        let engine = MultiModelEngine::with_models(true, false, true);
        let result = engine.query_time_series(0, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[test]
    fn test_query_graph() {
        let mut engine = MultiModelEngine::new();

        // Vertex data format:
        // label
        // edge1,edge2
        // prop1=val1
        let vertex_value = b"person\nalice,bob\ndepartment=engineering";
        engine
            .engine_mut()
            .set(b"graph/v1".to_vec(), vertex_value.to_vec())
            .unwrap();

        let vertex = engine.query_graph("v1").unwrap();
        assert_eq!(vertex.id, "v1");
        assert_eq!(vertex.label, "person");
        assert_eq!(vertex.edges, vec!["alice", "bob"]);
        assert_eq!(vertex.properties.get("department").unwrap(), "engineering");
    }

    #[test]
    fn test_query_graph_not_found() {
        let engine = MultiModelEngine::new();
        let vertex = engine.query_graph("nonexistent").unwrap();
        assert_eq!(vertex.id, "nonexistent");
        assert!(vertex.label.is_empty());
        assert!(vertex.edges.is_empty());
    }

    #[test]
    fn test_query_graph_disabled() {
        let engine = MultiModelEngine::with_models(true, true, false);
        let result = engine.query_graph("v1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[test]
    fn test_toggle_models() {
        let mut engine = MultiModelEngine::new();
        assert!(engine.is_document_enabled());
        engine.set_document_enabled(false);
        assert!(!engine.is_document_enabled());

        assert!(engine.is_time_series_enabled());
        engine.set_time_series_enabled(false);
        assert!(!engine.is_time_series_enabled());

        assert!(engine.is_graph_enabled());
        engine.set_graph_enabled(false);
        assert!(!engine.is_graph_enabled());
    }

    #[test]
    fn test_with_engine_custom() {
        let custom_engine = InMemoryEngine::new();
        let mut engine = MultiModelEngine::with_engine(Box::new(custom_engine));
        engine
            .engine_mut()
            .set(b"custom_key".to_vec(), b"field1=val1\nfield2=val2".to_vec())
            .unwrap();
        let doc = engine.query_document("custom_key").unwrap();
        assert_eq!(doc.get("field1").unwrap(), "val1");
        assert_eq!(doc.get("field2").unwrap(), "val2");
    }

    #[test]
    fn test_query_graph_with_edges_empty() {
        let mut engine = MultiModelEngine::new();

        // Vertex with no edges
        let vertex_value = b"server\n\nregion=us-east-1";
        engine
            .engine_mut()
            .set(b"graph/s1".to_vec(), vertex_value.to_vec())
            .unwrap();

        let vertex = engine.query_graph("s1").unwrap();
        assert_eq!(vertex.label, "server");
        assert!(vertex.edges.is_empty());
        assert_eq!(vertex.properties.get("region").unwrap(), "us-east-1");
    }

    #[test]
    fn test_query_time_series_no_label() {
        let mut engine = MultiModelEngine::new();

        // Value without label (no '|' separator)
        let ts: u128 = 5000;
        let key = format!("ts/{:032x}", ts);
        engine
            .engine_mut()
            .set(key.into_bytes(), b"42.0".to_vec())
            .unwrap();

        let points = engine.query_time_series(0, 10000).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].timestamp, ts);
        assert_eq!(points[0].value, 42.0);
        assert_eq!(points[0].label, None);
    }

    #[test]
    fn test_parse_document_empty() {
        let doc = parse_document(b"").unwrap();
        assert!(doc.is_empty());
    }

    #[test]
    fn test_parse_document_skip_empty_lines() {
        let doc = parse_document(b"a=1\n\nb=2\n").unwrap();
        assert_eq!(doc.get("a").unwrap(), "1");
        assert_eq!(doc.get("b").unwrap(), "2");
    }

    #[test]
    fn test_parse_graph_vertex_empty() {
        let vertex = parse_graph_vertex("empty", b"").unwrap();
        assert_eq!(vertex.id, "empty");
        assert!(vertex.label.is_empty());
    }

    #[test]
    fn test_parse_graph_vertex_label_only() {
        let vertex = parse_graph_vertex("v1", b"just_label").unwrap();
        assert_eq!(vertex.label, "just_label");
        assert!(vertex.edges.is_empty());
        assert!(vertex.properties.is_empty());
    }
}
