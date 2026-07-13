//! Built-in vector search / embeddings index.
//!
//! Provides a [`VectorIndex`] that stores dense vector embeddings alongside
//! string keys and supports approximate nearest-neighbour (ANN) search
//! via brute-force cosine similarity.
//!
//! # Status
//!
//! The index is fully functional with brute-force search (O(n) scan). For
//! large datasets, consider replacing the internal index with an HNSW graph,
//! IVF, or a dedicated ANN library (e.g. `usearch`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A dense vector embedding stored in the index.
type Embedding = Vec<f32>;

/// In-memory vector index for ANN search.
///
/// Stores (key, embedding) pairs and performs brute-force cosine similarity
/// search.  This is correct but slow for large datasets; replace the
/// internal index with an HNSW graph for production use.
#[derive(Serialize, Deserialize)]
pub struct VectorIndex {
    /// Key → embedding mapping.
    vectors: HashMap<String, Embedding>,
    /// Dimensionality of stored embeddings (all must match).
    dimension: usize,
}

impl VectorIndex {
    /// Create a new empty vector index with the given dimension.
    ///
    /// All embeddings inserted must have exactly `dimension` elements.
    pub fn new(dimension: usize) -> Self {
        Self {
            vectors: HashMap::new(),
            dimension,
        }
    }

    /// Insert or update a key with its embedding vector.
    ///
    /// Returns an error if the embedding length does not match the index
    /// dimension.
    pub fn insert(&mut self, key: &str, embedding: Embedding) -> Result<(), String> {
        if embedding.len() != self.dimension {
            return Err(format!(
                "embedding dimension mismatch: expected {} but got {}",
                self.dimension,
                embedding.len()
            ));
        }
        self.vectors.insert(key.to_string(), embedding);
        Ok(())
    }

    /// Search the index for the `k` nearest neighbours of `query`.
    ///
    /// Returns a list of keys sorted by descending cosine similarity
    /// (most similar first).  When there are fewer than `k` entries in the
    /// index, all entries are returned.
    ///
    /// The query embedding must match the index dimension.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<String>, String> {
        if query.len() != self.dimension {
            return Err(format!(
                "query dimension mismatch: expected {} but got {}",
                self.dimension,
                query.len()
            ));
        }

        if self.vectors.is_empty() {
            return Ok(Vec::new());
        }

        let query_norm = cosine_norm(query);
        if query_norm == 0.0 {
            return Err("zero-vector query cannot be normalised".to_string());
        }

        let mut scored: Vec<(f32, &String)> = self
            .vectors
            .iter()
            .map(|(key, vec)| {
                let sim = cosine_similarity(query, vec, query_norm);
                (sim, key)
            })
            .collect();

        // Sort by descending similarity.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored
            .into_iter()
            .take(k)
            .map(|(_, key)| key.clone())
            .collect())
    }

    /// Return the number of vectors stored in the index.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Returns `true` if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Return the dimension of stored embeddings.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Remove a key from the index.
    pub fn remove(&mut self, key: &str) -> Option<Embedding> {
        self.vectors.remove(key)
    }

    /// Clear all vectors from the index.
    pub fn clear(&mut self) {
        self.vectors.clear();
    }

    /// Save the index to disk as a JSON file.
    ///
    /// Serialises the entire index (keys and embeddings) to the given path.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string(self).map_err(|e| format!("serialization error: {}", e))?;
        fs::write(path, &json).map_err(|e| format!("write error: {}", e))
    }

    /// Load a previously saved index from a JSON file.
    ///
    /// The file must have been written by [`save`](Self::save).
    pub fn load(path: &Path) -> Result<Self, String> {
        let json = fs::read_to_string(path).map_err(|e| format!("read error: {}", e))?;
        serde_json::from_str(&json).map_err(|e| format!("deserialization error: {}", e))
    }
}

// ── Math helpers ──────────────────────────────────────────────────────────────

/// Compute the L2 norm of a vector.
fn cosine_norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Compute cosine similarity between two vectors.
///
/// `query_norm` is the pre-computed norm of `a`.
fn cosine_similarity(a: &[f32], b: &[f32], query_norm: f32) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let b_norm = cosine_norm(b);
    if b_norm == 0.0 {
        return 0.0;
    }
    dot / (query_norm * b_norm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_search() {
        let mut idx = VectorIndex::new(3);
        idx.insert("cat", vec![0.1, 0.2, 0.3]).unwrap();
        idx.insert("dog", vec![0.4, 0.5, 0.6]).unwrap();
        idx.insert("fish", vec![0.7, 0.8, 0.9]).unwrap();

        assert_eq!(idx.len(), 3);

        // Query close to "fish"
        let results = idx.search(&[0.69, 0.79, 0.89], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], "fish");
    }

    #[test]
    fn test_search_empty_index() {
        let idx = VectorIndex::new(4);
        let results = idx.search(&[1.0, 2.0, 3.0, 4.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_insert_dimension_mismatch() {
        let mut idx = VectorIndex::new(3);
        let result = idx.insert("bad", vec![1.0, 2.0]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("dimension mismatch"));
    }

    #[test]
    fn test_query_dimension_mismatch() {
        let mut idx = VectorIndex::new(3);
        idx.insert("a", vec![0.1, 0.2, 0.3]).unwrap();
        let result = idx.search(&[1.0, 2.0], 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_and_clear() {
        let mut idx = VectorIndex::new(2);
        idx.insert("x", vec![1.0, 0.0]).unwrap();
        idx.insert("y", vec![0.0, 1.0]).unwrap();
        assert_eq!(idx.len(), 2);

        idx.remove("x");
        assert_eq!(idx.len(), 1);

        idx.clear();
        assert!(idx.is_empty());
    }

    #[test]
    fn test_zero_vector_query() {
        let mut idx = VectorIndex::new(2);
        idx.insert("a", vec![1.0, 0.0]).unwrap();
        let result = idx.search(&[0.0, 0.0], 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.json");

        // Create and populate an index
        let mut idx = VectorIndex::new(3);
        idx.insert("cat", vec![0.1, 0.2, 0.3]).unwrap();
        idx.insert("dog", vec![0.4, 0.5, 0.6]).unwrap();
        idx.save(&path).unwrap();

        // Load a new index from disk
        let loaded = VectorIndex::load(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.dimension(), 3);

        // Search still works on the loaded index
        let results = loaded.search(&[0.35, 0.45, 0.55], 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "dog");
    }
}
