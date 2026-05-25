//! # Note Engine — Obsidian-like note-taking layer
//!
//! Provides a complete note-taking layer on top of the ApexStore LSM engine,
//! with support for:
//!
//! - **Wikilinks** — `[[Note Name]]` parsing and bidirectional linking
//! - **Tags** — `#tag` extraction and indexing
//! - **Frontmatter** — YAML metadata parsing (title, aliases, dates, custom fields)
//! - **Graph view** — Force-directed graph assembly for visualization
//! - **Forward/Backlink indexes** — Automatic bidirectional link management
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │                NoteEngine                    │
//! │  (wraps LsmEngine + manages indexes)         │
//! │                                              │
//! │  ┌──────────┐  ┌─────────┐  ┌───────────┐   │
//! │  │NoteParser│  │NoteIndex│  │NoteGraph   │   │
//! │  │wikilinks │  │link:    │  │D3.js JSON  │   │
//! │  │tags      │  │backlink:│  │traversal   │   │
//! │  │frontmatter│ └─────────┘  └───────────┘   │
//! │  └──────────┘                               │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! # Storage Schema
//!
//! ```text
//! cf "default":
//!   note:/path/to/note.md        → Markdown content
//!   link:{target_note}           → [source_notes...] (JSON array)
//!   backlink:{source_note}       → [target_notes...] (JSON array)
//!   tag:{tagname}                → [note_paths...] (JSON array)
//!   __blob_meta:{name}           → Blob metadata (JSON)
//!   __blob_chunk:{name}:{seq}    → Raw chunk data
//! ```

pub mod graph;
pub mod index;
pub mod parser;

use crate::infra::error::Result;
use crate::storage::cache::Cache;

// Re-exports
pub use graph::{GraphConfig, GraphData, GraphDepth, GraphEdge, GraphNode, NoteGraph};
pub use index::NoteIndex;
pub use parser::{
    parse_frontmatter, parse_note, parse_tags, parse_wikilinks, Frontmatter, LinkType, ParsedNote,
    Wikilink,
};

/// Type alias for the note engine using the default LSM engine with `GlobalBlockCache`.
pub type NotesEngine = NoteEngine<std::sync::Arc<crate::storage::cache::GlobalBlockCache>>;

/// The high-level note engine that wraps the LSM storage engine and provides
/// Obsidian-like note operations.
///
/// # Type Parameters
///
/// * `C` — The block cache implementation (typically `GlobalBlockCache`).
pub struct NoteEngine<C: Cache> {
    /// Reference to the underlying LSM engine.
    engine: std::sync::Arc<crate::core::engine::Engine<C>>,
    /// Column family used for note/index storage.
    cf: String,
}

impl<C: Cache> NoteEngine<C> {
    /// Create a new `NoteEngine` wrapping an existing LSM engine.
    pub fn new(engine: std::sync::Arc<crate::core::engine::Engine<C>>) -> Self {
        Self {
            engine,
            cf: "default".to_string(),
        }
    }

    /// Create a `NoteEngine` with a custom column family.
    pub fn with_cf(engine: std::sync::Arc<crate::core::engine::Engine<C>>, cf: &str) -> Self {
        Self {
            engine,
            cf: cf.to_string(),
        }
    }

    /// Return a reference to the underlying LSM engine.
    pub fn engine(&self) -> &crate::core::engine::Engine<C> {
        &self.engine
    }

    /// Return the column family used for note storage.
    pub fn cf(&self) -> &str {
        &self.cf
    }

    // ── Note CRUD ──────────────────────────────────────────────────────

    /// Create or update a note. Automatically parses wikilinks and tags from
    /// the content and updates the link/tag indexes.
    pub fn put_note(&self, path: &str, content: &str) -> Result<()> {
        let note_key = format!("note:{}", path);

        // Parse the full note content
        let parsed = parse_note(content);

        // Store the note content
        self.engine
            .put_cf(&self.cf, note_key.into_bytes(), content.as_bytes().to_vec())?;

        // Extract link targets (only WikiLink and BlockRef types)
        let link_targets: Vec<String> = parsed
            .links
            .iter()
            .filter(|l| {
                matches!(
                    l.link_type,
                    parser::LinkType::WikiLink | parser::LinkType::BlockRef
                )
            })
            .map(|l| l.target.clone())
            .collect();

        // Update link indexes
        NoteIndex::index_links(&self.engine, &self.cf, path, &link_targets)?;

        // Update tag indexes
        self.index_tags(path, &parsed.inline_tags)?;

        Ok(())
    }

    /// Create or update a note and save a version snapshot.
    /// Same as `put_note` but also records a version entry for history.
    pub fn put_note_with_version(&self, path: &str, content: &str) -> Result<()> {
        self.put_note(path, content)?;
        self.save_version(path)
    }

    /// Save the current note content as a version entry.
    fn save_version(&self, path: &str) -> Result<()> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros();

        let version_key = format!("__version:{}", path);
        let version_entry_key = format!("__version_content:{}:{}", path, ts);

        // Get current content
        let content = self.get_note(path)?;
        if let Some(content) = content {
            // Store the versioned content
            self.engine.put_cf(
                &self.cf,
                version_entry_key.into_bytes(),
                content.into_bytes(),
            )?;

            // Update version metadata
            let mut timestamps: Vec<u128> =
                match self.engine.get_cf(&self.cf, version_key.as_bytes())? {
                    Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                    None => Vec::new(),
                };
            timestamps.push(ts);

            // Trim to max 100 versions
            while timestamps.len() > 100 {
                let removed = timestamps.remove(0);
                let old_key = format!("__version_content:{}:{}", path, removed);
                let _ = self.engine.delete_cf(&self.cf, old_key.into_bytes());
            }

            let value = serde_json::to_vec(&timestamps).map_err(|e| {
                crate::infra::error::LsmError::InvalidArgument(format!("JSON error: {}", e))
            })?;
            self.engine
                .put_cf(&self.cf, version_key.into_bytes(), value)?;
        }

        Ok(())
    }

    /// Get the version history for a note (list of timestamps).
    pub fn get_version_history(&self, path: &str) -> Result<Vec<u128>> {
        let version_key = format!("__version:{}", path);
        match self.engine.get_cf(&self.cf, version_key.as_bytes())? {
            Some(bytes) => {
                let timestamps: Vec<u128> = serde_json::from_slice(&bytes).unwrap_or_default();
                Ok(timestamps)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Get the content of a note at a specific version timestamp.
    pub fn get_note_at_version(&self, path: &str, timestamp: u128) -> Result<Option<String>> {
        let version_entry_key = format!("__version_content:{}:{}", path, timestamp);
        match self.engine.get_cf(&self.cf, version_entry_key.as_bytes())? {
            Some(bytes) => {
                let content = String::from_utf8_lossy(&bytes).to_string();
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }

    /// Remove a specific version from history.
    pub fn remove_version(&self, path: &str, timestamp: u128) -> Result<bool> {
        let version_key = format!("__version:{}", path);
        let version_entry_key = format!("__version_content:{}:{}", path, timestamp);

        // Remove the version content
        self.engine
            .delete_cf(&self.cf, version_entry_key.into_bytes())?;

        // Update the version metadata
        let mut timestamps: Vec<u128> =
            match self.engine.get_cf(&self.cf, version_key.as_bytes())? {
                Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                None => return Ok(false),
            };

        let before = timestamps.len();
        timestamps.retain(|t| *t != timestamp);

        if timestamps.is_empty() {
            self.engine.delete_cf(&self.cf, version_key.into_bytes())?;
            Ok(before != 0)
        } else {
            let value = serde_json::to_vec(&timestamps).map_err(|e| {
                crate::infra::error::LsmError::InvalidArgument(format!("JSON error: {}", e))
            })?;
            self.engine
                .put_cf(&self.cf, version_key.into_bytes(), value)?;
            Ok(true)
        }
    }

    /// Restore a note to a previous version. Saves current version first, then
    /// overwrites with the old version's content.
    pub fn restore_version(&self, path: &str, timestamp: u128) -> Result<bool> {
        // Get the old content
        let content = self.get_note_at_version(path, timestamp)?;
        match content {
            Some(old_content) => {
                // Save current as a version first
                self.save_version(path)?;
                // Write the old content as current
                self.put_note(path, &old_content)?;
                // Save another version entry for the restore
                self.save_version(path)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Get all notes that were active at the given timestamp using TimeTravelEngine.
    pub fn get_notes_at_timestamp(
        &self,
        time_travel: &crate::infra::time_travel::TimeTravelEngine,
        timestamp: u128,
    ) -> Result<Vec<(String, String)>> {
        // Query all note: prefixed keys at the given timestamp
        // Since TimeTravelEngine doesn't have prefix scan, we iterate known notes
        let current_notes = self.list_notes(None)?;
        let mut result = Vec::new();
        for path in &current_notes {
            let key = format!("note:{}", path);
            if let Some(content) = time_travel.query_as_of(key.as_bytes(), timestamp) {
                result.push((path.clone(), String::from_utf8_lossy(&content).to_string()));
            }
        }
        Ok(result)
    }

    /// Get a note's content by path.
    pub fn get_note(&self, path: &str) -> Result<Option<String>> {
        let note_key = format!("note:{}", path);
        match self.engine.get_cf(&self.cf, note_key.into_bytes())? {
            Some(bytes) => {
                let content = String::from_utf8_lossy(&bytes).to_string();
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }

    /// Delete a note and clean up all its indexes.
    pub fn delete_note(&self, path: &str) -> Result<()> {
        let note_key = format!("note:{}", path);

        // Remove link indexes
        NoteIndex::remove_note_links(&self.engine, &self.cf, path)?;

        // Remove tag indexes
        self.remove_note_tags(path)?;

        // Delete the note content
        self.engine.delete_cf(&self.cf, note_key.into_bytes())?;

        Ok(())
    }

    /// Rename a note from `old_path` to `new_path`.
    pub fn rename_note(&self, old_path: &str, new_path: &str) -> Result<()> {
        // Get existing content
        let content = self.get_note(old_path)?;
        let content = match content {
            Some(c) => c,
            None => {
                return Err(crate::infra::error::LsmError::InvalidArgument(format!(
                    "Note not found: {}",
                    old_path
                )));
            }
        };

        // Parse to get new links/tags
        let parsed = parse_note(&content);
        let link_targets: Vec<String> = parsed
            .links
            .iter()
            .filter(|l| {
                matches!(
                    l.link_type,
                    parser::LinkType::WikiLink | parser::LinkType::BlockRef
                )
            })
            .map(|l| l.target.clone())
            .collect();

        // Use NoteIndex to rename
        NoteIndex::rename_note(&self.engine, &self.cf, old_path, new_path, &link_targets)?;

        // Store content under new path
        let new_note_key = format!("note:{}", new_path);
        self.engine.put_cf(
            &self.cf,
            new_note_key.into_bytes(),
            content.as_bytes().to_vec(),
        )?;

        // Delete old note content
        let old_note_key = format!("note:{}", old_path);
        self.engine.delete_cf(&self.cf, old_note_key.into_bytes())?;

        Ok(())
    }

    /// List all notes, optionally filtered by a prefix.
    pub fn list_notes(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let search_prefix = match prefix {
            Some(p) => format!("note:{}", p),
            None => "note:".to_string(),
        };

        let (results, _cursor) =
            self.engine
                .search_prefix(&search_prefix, None, crate::core::engine::MAX_SCAN_LIMIT)?;

        let paths: Vec<String> = results
            .into_iter()
            .map(|(k, _)| {
                let key = String::from_utf8_lossy(&k).to_string();
                key.strip_prefix("note:").unwrap_or(&key).to_string()
            })
            .collect();

        Ok(paths)
    }

    // ── Tag management ─────────────────────────────────────────────────

    /// Index tags for a note.
    fn index_tags(&self, note_path: &str, tags: &[String]) -> Result<()> {
        for tag in tags {
            let tag_key = format!("tag:{}", tag);

            // Get existing notes with this tag
            let mut notes: Vec<String> = match self.engine.get_cf(&self.cf, tag_key.as_bytes())? {
                Some(bytes) => {
                    let value = String::from_utf8_lossy(&bytes);
                    serde_json::from_str(&value).unwrap_or_default()
                }
                None => Vec::new(),
            };

            // Add this note if not already present
            if !notes.contains(&note_path.to_string()) {
                notes.push(note_path.to_string());
                let value = serde_json::to_string(&notes).map_err(|e| {
                    crate::infra::error::LsmError::InvalidArgument(format!("JSON error: {}", e))
                })?;
                self.engine
                    .put_cf(&self.cf, tag_key.into_bytes(), value.into_bytes())?;
            }
        }
        Ok(())
    }

    /// Remove all tag indexes for a note.
    fn remove_note_tags(&self, note_path: &str) -> Result<()> {
        // Scan for tags containing this note
        // Since we don't have a reverse tag index, we look up known tags
        // via prefix scan
        let (results, _cursor) =
            self.engine
                .search_prefix("tag:", None, crate::core::engine::MAX_SCAN_LIMIT)?;

        for (key, value) in &results {
            let mut notes: Vec<String> =
                serde_json::from_str(&String::from_utf8_lossy(value)).unwrap_or_default();

            if notes.contains(&note_path.to_string()) {
                notes.retain(|n| n != note_path);

                if notes.is_empty() {
                    self.engine.delete_cf(&self.cf, key.clone())?;
                } else {
                    let new_value = serde_json::to_string(&notes).map_err(|e| {
                        crate::infra::error::LsmError::InvalidArgument(format!("JSON error: {}", e))
                    })?;
                    self.engine
                        .put_cf(&self.cf, key.clone(), new_value.into_bytes())?;
                }
            }
        }

        Ok(())
    }

    /// Get all notes that have a specific tag.
    pub fn get_notes_by_tag(&self, tag: &str) -> Result<Vec<String>> {
        let tag_key = format!("tag:{}", tag);
        match self.engine.get_cf(&self.cf, tag_key.into_bytes())? {
            Some(bytes) => {
                let value = String::from_utf8_lossy(&bytes);
                let notes: Vec<String> = serde_json::from_str(&value).unwrap_or_default();
                Ok(notes)
            }
            None => Ok(Vec::new()),
        }
    }

    /// List all tags with note counts.
    pub fn list_tags(&self) -> Result<Vec<(String, usize)>> {
        let (results, _cursor) =
            self.engine
                .search_prefix("tag:", None, crate::core::engine::MAX_SCAN_LIMIT)?;

        let mut tags = Vec::new();
        for (key, value) in &results {
            let key_str = String::from_utf8_lossy(key).to_string();
            if let Some(tag_name) = key_str.strip_prefix("tag:") {
                let notes: Vec<String> =
                    serde_json::from_str(&String::from_utf8_lossy(value)).unwrap_or_default();
                tags.push((tag_name.to_string(), notes.len()));
            }
        }

        Ok(tags)
    }

    /// Search notes by tag with cursor-based pagination.
    ///
    /// Returns a list of note paths and an optional cursor for the next page.
    pub fn search_by_tag(
        &self,
        tag: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<String>, Option<String>)> {
        let (results, next_cursor) =
            self.engine
                .search_prefix(&format!("tag:{}", tag), cursor, limit)?;

        let note_paths: Vec<String> = results
            .into_iter()
            .filter_map(|(k, v)| {
                let key_str = String::from_utf8_lossy(&k).to_string();
                if key_str == format!("tag:{}", tag) {
                    // The direct tag entry — parse its value array
                    let notes: Vec<String> =
                        serde_json::from_str(&String::from_utf8_lossy(&v)).unwrap_or_default();
                    Some(notes)
                } else {
                    None
                }
            })
            .flatten()
            .collect();

        Ok((
            note_paths,
            next_cursor.map(|c| String::from_utf8_lossy(c.as_ref()).to_string()),
        ))
    }

    // ── Link index queries ──────────────────────────────────────────────

    /// Get all notes that link TO the given note (incoming links).
    pub fn get_backlinks(&self, note_path: &str) -> Result<Vec<String>> {
        NoteIndex::get_backlinks(&self.engine, &self.cf, note_path)
    }

    /// Get all notes that the given note links TO (outgoing links).
    pub fn get_forward_links(&self, note_path: &str) -> Result<Vec<String>> {
        NoteIndex::get_forward_links(&self.engine, &self.cf, note_path)
    }

    // ── Graph ──────────────────────────────────────────────────────────

    /// Build a graph centered on a specific note.
    pub fn build_graph(&self, center_note: &str, config: &GraphConfig) -> Result<GraphData> {
        NoteGraph::build_graph(&self.engine, &self.cf, center_note, config)
    }

    // ─── Snapshot / version history ───────────────────────────────────

    /// Create a manual snapshot of the current note state.
    pub fn create_snapshot(
        &self,
        label: &str,
        time_travel: &mut crate::infra::time_travel::TimeTravelEngine,
    ) -> u128 {
        // Collect all notes
        let notes = self.list_notes(None).unwrap_or_default();
        let mut data = std::collections::HashMap::new();

        for path in &notes {
            if let Ok(Some(content)) = self.get_note(path) {
                let key = format!("note:{}", path);
                data.insert(key.into_bytes(), content.into_bytes());
            }
        }

        time_travel.capture(data, label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::storage::cache::GlobalBlockCache;
    use std::sync::Arc;

    fn create_note_engine() -> NoteEngine<Arc<GlobalBlockCache>> {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Arc::new(
            crate::core::engine::Engine::new_from_config(&config, GlobalBlockCache::new(10, 4096))
                .unwrap(),
        );
        NoteEngine::new(engine)
    }

    #[test]
    fn test_put_and_get_note() {
        let engine = create_note_engine();
        engine.put_note("test-note", "# Hello World").unwrap();

        let content = engine.get_note("test-note").unwrap();
        assert_eq!(content, Some("# Hello World".to_string()));
    }

    #[test]
    fn test_get_nonexistent_note() {
        let engine = create_note_engine();
        let content = engine.get_note("nonexistent").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn test_delete_note() {
        let engine = create_note_engine();
        engine.put_note("to-delete", "content").unwrap();
        engine.delete_note("to-delete").unwrap();

        let content = engine.get_note("to-delete").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn test_rename_note() {
        let engine = create_note_engine();
        engine.put_note("old-name", "# Hello").unwrap();
        engine.rename_note("old-name", "new-name").unwrap();

        let old = engine.get_note("old-name").unwrap();
        assert!(old.is_none());

        let new = engine.get_note("new-name").unwrap();
        assert_eq!(new, Some("# Hello".to_string()));
    }

    #[test]
    fn test_list_notes() {
        let engine = create_note_engine();
        engine.put_note("doc/a", "Note A").unwrap();
        engine.put_note("doc/b", "Note B").unwrap();
        engine.put_note("other/c", "Note C").unwrap();

        let all = engine.list_notes(None).unwrap();
        assert_eq!(all.len(), 3);

        let filtered = engine.list_notes(Some("doc/")).unwrap();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_note_with_wikilinks_indexes_links() {
        let engine = create_note_engine();
        let content = "See [[target-note]] and [[another|Display]]";
        engine.put_note("source-note", content).unwrap();

        // Check forward links
        let forward =
            NoteIndex::get_forward_links(engine.engine(), "default", "source-note").unwrap();
        assert!(forward.contains(&"target-note".to_string()));
        assert!(forward.contains(&"another".to_string()));

        // Check backlinks from target perspective
        let backlinks =
            NoteIndex::get_backlinks(engine.engine(), "default", "target-note").unwrap();
        assert!(backlinks.contains(&"source-note".to_string()));
    }

    #[test]
    fn test_note_with_tags() {
        let engine = create_note_engine();
        let content = "# Hello\n\nThis is #important and #rust";
        engine.put_note("tagged-note", content).unwrap();

        let notes = engine.get_notes_by_tag("important").unwrap();
        assert!(notes.contains(&"tagged-note".to_string()));

        let notes = engine.get_notes_by_tag("rust").unwrap();
        assert!(notes.contains(&"tagged-note".to_string()));
    }

    #[test]
    fn test_list_tags() {
        let engine = create_note_engine();
        engine.put_note("note-a", "#tag1 content").unwrap();
        engine.put_note("note-b", "#tag1 and #tag2").unwrap();

        let tags = engine.list_tags().unwrap();
        assert!(tags.contains(&("tag1".to_string(), 2)));
        assert!(tags.contains(&("tag2".to_string(), 1)));
    }

    #[test]
    fn test_note_with_frontmatter() {
        let engine = create_note_engine();
        let content = "---\ntitle: My Note\ntags: [frontmatter-tag]\n---\n\nBody with #inline-tag";
        engine.put_note("fm-note", content).unwrap();

        // Tags from both frontmatter and inline should be indexed
        let notes = engine.get_notes_by_tag("frontmatter-tag").unwrap();
        assert!(notes.contains(&"fm-note".to_string()));

        let notes = engine.get_notes_by_tag("inline-tag").unwrap();
        assert!(notes.contains(&"fm-note".to_string()));
    }

    #[test]
    fn test_graph_from_engine() {
        let engine = create_note_engine();
        engine
            .put_note("note-a", "Links to [[note-b]] and [[note-c]]")
            .unwrap();
        engine.put_note("note-b", "Links to [[note-c]]").unwrap();
        engine.put_note("note-c", "Orphan").unwrap();

        let graph = engine
            .build_graph("note-a", &GraphConfig::default())
            .unwrap();
        assert!(graph.nodes.len() >= 2);
        assert_eq!(graph.root, "note-a");
    }

    #[test]
    fn test_create_snapshot() {
        let engine = create_note_engine();
        engine.put_note("snap-note", "Snapshot content").unwrap();

        let mut time_travel = crate::infra::time_travel::TimeTravelEngine::new(10);
        let ts = engine.create_snapshot("test-snapshot", &mut time_travel);

        assert_eq!(time_travel.snapshot_count(), 1);
        let snapshots = time_travel.list_snapshots();
        assert_eq!(snapshots[0].1, "test-snapshot");

        // Snapshot should contain the note
        let data = time_travel.query_as_of(b"note:snap-note" as &[u8], ts + 1);
        assert!(data.is_some());
    }

    #[test]
    fn test_rename_updates_indexes() {
        let engine = create_note_engine();

        // Note-X links to Note-Y
        engine.put_note("note-x", "See [[note-y]]").unwrap();

        // Note-Z links to Note-X
        engine.put_note("note-z", "See [[note-x]]").unwrap();

        // Rename Note-X
        engine.rename_note("note-x", "note-x-renamed").unwrap();

        // Note-Z should now link to Note-X-renamed
        let backlinks =
            NoteIndex::get_backlinks(engine.engine(), "default", "note-x-renamed").unwrap();
        assert!(backlinks.contains(&"note-z".to_string()));

        // Old Note-X should have no backlinks
        let old_backlinks = NoteIndex::get_backlinks(engine.engine(), "default", "note-x").unwrap();
        assert!(old_backlinks.is_empty());
    }
}
