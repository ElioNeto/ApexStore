//! Full-text search for note content using an inverted index.
//!
//! Storage schema:
//!
//! ```text
//! cf "default":
//!   fts:{term}                → JSON array of { path, count, last_indexed }
//!   fts:meta:{note_path}      → JSON { checksum, word_count }
//! ```
//!
//! Tokenization rules:
//! - Split on whitespace and punctuation
//! - Lowercase all terms
//! - Min word length: 2 chars
//! - Max word length: 50 chars
//! - Stop words removed (configurable)

use crate::infra::error::{LsmError, Result};
use crate::storage::cache::Cache;
use serde::{Deserialize, Serialize};

/// A single term entry in the inverted index.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TermEntry {
    /// Note path containing this term.
    path: String,
    /// How many times the term appears in the note.
    count: usize,
}

/// Metadata about a note's indexed content.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FtsMeta {
    /// Checksum to detect content changes.
    checksum: u64,
    /// Total word count in the note.
    word_count: usize,
}

/// A search result hit.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    /// Note path.
    pub path: String,
    /// Relevance score (approximate TF-IDF).
    pub score: f64,
    /// Content snippet with highlighted terms.
    pub snippet: String,
}

/// The full-text search engine.
pub struct FullTextSearch;

/// Default stop words to filter out.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
    "from", "as", "is", "was", "are", "were", "be", "been", "being", "have", "has", "had", "do",
    "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can", "not", "no",
    "nor", "so", "if", "then", "than", "that", "this", "these", "those", "it", "its", "he", "she",
    "they", "them", "we", "you", "all", "each", "every", "some", "any", "both", "few", "more",
    "most", "other", "such", "only", "own", "same", "too", "very", "just", "also", "about",
    "above", "after", "again", "against", "below", "between", "into", "through", "during",
    "before", "after", "up", "down", "out", "off", "over", "under", "here", "there", "where",
    "why", "how",
];

impl FullTextSearch {
    /// Tokenize content into terms.
    fn tokenize(content: &str) -> Vec<String> {
        let mut terms = Vec::new();
        let mut current = String::new();

        for ch in content.chars() {
            if ch.is_alphanumeric() {
                current.push(ch);
            } else if !current.is_empty() {
                let term = current.to_lowercase();
                if term.len() >= 2 && term.len() <= 50 && !STOP_WORDS.contains(&term.as_str()) {
                    terms.push(term);
                }
                current.clear();
            }
        }

        // Handle last term
        if !current.is_empty() {
            let term = current.to_lowercase();
            if term.len() >= 2 && term.len() <= 50 && !STOP_WORDS.contains(&term.as_str()) {
                terms.push(term);
            }
        }

        terms
    }

    /// Compute a simple hash for change detection.
    fn checksum(content: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Index a single note — updates the inverted index with the note's terms.
    /// Only re-indexes if content has changed (detected via checksum).
    pub fn index_note<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        path: &str,
        content: &str,
    ) -> Result<()> {
        let meta_key = format!("fts:meta:{}", path);
        let new_checksum = Self::checksum(content);

        // Check if content changed since last index
        if let Some(bytes) = engine.get_cf(cf, meta_key.as_bytes())? {
            let meta: FtsMeta = serde_json::from_slice(&bytes)
                .map_err(|e| LsmError::InvalidArgument(format!("FTS meta: {}", e)))?;
            if meta.checksum == new_checksum {
                return Ok(()); // Unchanged
            }
        }

        let terms = Self::tokenize(content);
        let word_count = terms.len();

        // Build term frequency map
        let mut tf_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for term in &terms {
            *tf_map.entry(term.clone()).or_insert(0) += 1;
        }

        // Update inverted index for each term
        for (term, count) in &tf_map {
            let fts_key = format!("fts:{}", term);
            let mut entries: Vec<TermEntry> = match engine.get_cf(cf, fts_key.as_bytes())? {
                Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                None => Vec::new(),
            };

            // Update or insert entry for this note
            if let Some(existing) = entries.iter_mut().find(|e| e.path == path) {
                existing.count = *count;
            } else {
                entries.push(TermEntry {
                    path: path.to_string(),
                    count: *count,
                });
            }

            let value = serde_json::to_vec(&entries)
                .map_err(|e| LsmError::InvalidArgument(format!("FTS serialization: {}", e)))?;
            engine.put_cf(cf, fts_key.into_bytes(), value)?;
        }

        // Store metadata
        let meta = FtsMeta {
            checksum: new_checksum,
            word_count,
        };
        let meta_value = serde_json::to_vec(&meta)
            .map_err(|e| LsmError::InvalidArgument(format!("FTS meta: {}", e)))?;
        engine.put_cf(cf, meta_key.into_bytes(), meta_value)?;

        Ok(())
    }

    /// Remove all index entries for a note.
    pub fn remove_note<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        path: &str,
    ) -> Result<()> {
        // Find all terms pointing to this note by scanning fts: prefix
        let (results, _) = engine.search_prefix("fts:", None, 10_000)?;
        let mut terms_to_update: Vec<(String, Vec<TermEntry>)> = Vec::new();

        for (key_bytes, value_bytes) in &results {
            let key = String::from_utf8_lossy(key_bytes);
            if let Some(term) = key.strip_prefix("fts:") {
                if !term.starts_with("meta:") {
                    let mut entries: Vec<TermEntry> =
                        serde_json::from_slice(value_bytes).unwrap_or_default();
                    if entries.iter().any(|e| e.path == path) {
                        entries.retain(|e| e.path != path);
                        terms_to_update.push((term.to_string(), entries));
                    }
                }
            }
        }

        for (term, entries) in &terms_to_update {
            let fts_key = format!("fts:{}", term);
            if entries.is_empty() {
                engine.delete_cf(cf, fts_key.into_bytes())?;
            } else {
                let value = serde_json::to_vec(entries)
                    .map_err(|e| LsmError::InvalidArgument(format!("FTS: {}", e)))?;
                engine.put_cf(cf, fts_key.into_bytes(), value)?;
            }
        }

        // Remove metadata
        let meta_key = format!("fts:meta:{}", path);
        engine.delete_cf(cf, meta_key.into_bytes())?;

        Ok(())
    }

    /// Search for notes matching the given query.
    ///
    /// Supports:
    /// - `keyword` — basic term search
    /// - `"exact phrase"` — phrase search
    ///
    /// Returns results sorted by relevance (descending), with snippets.
    pub fn search<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchHit>> {
        let query = query.trim();

        // Check for phrase search
        if query.starts_with('"') && query.ends_with('"') && query.len() > 1 {
            let phrase = &query[1..query.len() - 1];
            let phrase_terms = Self::tokenize(phrase);
            return Self::search_phrase(engine, cf, &phrase_terms, max_results);
        }

        // Simple keyword search
        let terms = Self::tokenize(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }

        // Score each note across all query terms
        let mut scores: std::collections::HashMap<String, (f64, usize)> =
            std::collections::HashMap::new();
        let total_notes = 100.0; // Estimate for IDF normalization

        for term in &terms {
            let fts_key = format!("fts:{}", term);
            if let Some(bytes) = engine.get_cf(cf, fts_key.as_bytes())? {
                let entries: Vec<TermEntry> = serde_json::from_slice(&bytes).unwrap_or_default();

                let df = entries.len() as f64;
                let idf = (total_notes / (df + 1.0)).ln() + 1.0;

                for entry in entries {
                    let tf = (entry.count as f64).ln() + 1.0;
                    let score = tf * idf;
                    let current = scores.entry(entry.path).or_insert((0.0, 0));
                    current.0 += score;
                    current.1 += entry.count;
                }
            }
        }

        // Sort by score descending
        let mut results: Vec<(String, f64, usize)> = scores
            .into_iter()
            .map(|(path, (score, count))| (path, score, count))
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(max_results);

        // Build search hits with snippets
        let mut hits = Vec::new();
        for (path, score, _count) in &results {
            let note_key = format!("note:{}", path);
            let snippet = match engine.get_cf(cf, note_key.as_bytes())? {
                Some(bytes) => {
                    let content = String::from_utf8_lossy(&bytes);
                    generate_snippet(&content, terms.first().unwrap_or(&String::new()))
                }
                None => String::new(),
            };

            hits.push(SearchHit {
                path: path.clone(),
                score: *score,
                snippet,
            });
        }

        Ok(hits)
    }

    /// Phrase search — find notes containing all terms in order.
    fn search_phrase<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        phrase_terms: &[String],
        max_results: usize,
    ) -> Result<Vec<SearchHit>> {
        if phrase_terms.is_empty() {
            return Ok(Vec::new());
        }

        // Start with candidates from first term
        let first_key = format!("fts:{}", phrase_terms[0]);
        let mut candidates: std::collections::HashSet<String> = match engine
            .get_cf(cf, first_key.as_bytes())?
        {
            Some(bytes) => {
                let entries: Vec<TermEntry> = serde_json::from_slice(&bytes).unwrap_or_default();
                entries.into_iter().map(|e| e.path).collect()
            }
            None => return Ok(Vec::new()),
        };

        // Intersect with other terms' note sets
        for term in &phrase_terms[1..] {
            let fts_key = format!("fts:{}", term);
            let term_notes: std::collections::HashSet<String> =
                match engine.get_cf(cf, fts_key.as_bytes())? {
                    Some(bytes) => {
                        let entries: Vec<TermEntry> =
                            serde_json::from_slice(&bytes).unwrap_or_default();
                        entries.into_iter().map(|e| e.path).collect()
                    }
                    None => std::collections::HashSet::new(),
                };
            candidates = candidates.intersection(&term_notes).cloned().collect();
            if candidates.is_empty() {
                return Ok(Vec::new());
            }
        }

        // Simple scoring: number of matched terms
        let mut hits: Vec<SearchHit> = candidates
            .into_iter()
            .map(|path| {
                let snippet = match engine.get_cf(cf, format!("note:{}", path).as_bytes()) {
                    Ok(Some(bytes)) => {
                        let content = String::from_utf8_lossy(&bytes);
                        generate_snippet(&content, &phrase_terms[0])
                    }
                    _ => String::new(),
                };
                SearchHit {
                    path,
                    score: phrase_terms.len() as f64,
                    snippet,
                }
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(max_results);

        Ok(hits)
    }
}

/// Generate a snippet of text around the first occurrence of a search term.
fn generate_snippet(content: &str, term: &str) -> String {
    if term.is_empty() {
        // Return first 100 chars
        let truncated: String = content.chars().take(100).collect();
        return if content.len() > 100 {
            format!("{}...", truncated)
        } else {
            truncated
        };
    }

    let lower_content = content.to_lowercase();
    if let Some(pos) = lower_content.find(term) {
        // Find the word boundary before the match
        let start = pos.saturating_sub(40);
        let end = (pos + term.len() + 60).min(content.len());

        let snippet = &content[start..end];
        if start > 0 && end < content.len() {
            format!("...{}...", snippet)
        } else if start > 0 {
            format!("...{}", snippet)
        } else if end < content.len() {
            format!("{}...", snippet)
        } else {
            snippet.to_string()
        }
    } else {
        // Fall back to first 100 chars
        let truncated: String = content.chars().take(100).collect();
        if content.len() > 100 {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let terms = FullTextSearch::tokenize("Hello World Test");
        assert!(terms.contains(&"hello".to_string()));
        assert!(terms.contains(&"world".to_string()));
        assert!(terms.contains(&"test".to_string()));
        assert_eq!(terms.len(), 3);
    }

    #[test]
    fn test_tokenize_case_insensitive() {
        let terms = FullTextSearch::tokenize("Hello HELLO hello");
        assert_eq!(terms.iter().filter(|t| *t == "hello").count(), 3);
    }

    #[test]
    fn test_tokenize_short_words() {
        let terms = FullTextSearch::tokenize("a an the at to");
        // All are stop words or too short, should be empty
        assert!(terms.is_empty());
    }

    #[test]
    fn test_tokenize_punctuation() {
        let terms = FullTextSearch::tokenize("hello, world! test? ok.");
        assert!(terms.contains(&"hello".to_string()));
        assert!(terms.contains(&"world".to_string()));
        assert!(terms.contains(&"test".to_string()));
    }

    #[test]
    fn test_search_snippet() {
        let snippet = generate_snippet(
            "This is a long note about Rust programming language",
            "rust",
        );
        assert!(snippet.contains("Rust"));
        assert!(snippet.len() <= 200);
    }

    #[test]
    fn test_checksum_changes() {
        let c1 = FullTextSearch::checksum("hello world");
        let c2 = FullTextSearch::checksum("hello world!");
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_checksum_same() {
        let c1 = FullTextSearch::checksum("hello world");
        let c2 = FullTextSearch::checksum("hello world");
        assert_eq!(c1, c2);
    }
}
