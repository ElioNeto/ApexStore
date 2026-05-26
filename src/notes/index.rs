//! Forward-link and backlink index management for notes.
//!
//! Maintains bidirectional link indexes in the LSM store:
//!
//! - `link:{target_note}` → JSON array of source note paths that link TO target
//! - `backlink:{source_note}` → JSON array of target note paths linked FROM source

use crate::infra::error::{LsmError, Result};
use crate::storage::cache::Cache;

/// Key prefix for the forward-link index.
const LINK_PREFIX: &str = "link:";
/// Key prefix for the backlink index.
const BACKLINK_PREFIX: &str = "backlink:";

/// The link index engine — manages bidirectional link indexes.
///
/// This does NOT own the engine; it takes a reference for each operation.
/// `C` is the block cache type parameter of the LSM engine.
pub struct NoteIndex;

/// A diff of link changes for a single note update.
pub struct LinkDiff {
    /// Links that were added (new targets).
    pub added: Vec<String>,
    /// Links that were removed (old targets no longer present).
    pub removed: Vec<String>,
}

impl NoteIndex {
    /// Index the links for a note. Computes a diff between old and new links,
    /// then atomically updates both `link:` and `backlink:` indexes.
    ///
    /// Parameters:
    /// - `engine` — the LSM storage engine
    /// - `cf` — column family to use for index storage (default: "default")
    /// - `note_path` — the path of the note being indexed
    /// - `new_targets` — the new set of link targets extracted from the note
    pub fn index_links<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        note_path: &str,
        new_targets: &[String],
    ) -> Result<LinkDiff> {
        let current_targets = Self::get_forward_links(engine, cf, note_path)?;
        let diff = Self::compute_link_diff(&current_targets, new_targets);

        // Remove old links
        for target in &diff.removed {
            Self::remove_from_link_index(engine, cf, target, note_path)?;
        }

        // Add new links
        for target in &diff.added {
            Self::add_to_link_index(engine, cf, target, note_path)?;
        }

        // Update the backlink index for this note (store current outbound targets)
        let backlink_key = format!("{}{}", BACKLINK_PREFIX, note_path);
        let value = serde_json::to_string(new_targets)
            .map_err(|e| LsmError::InvalidArgument(format!("JSON serialization error: {}", e)))?;
        engine.put_cf(cf, backlink_key.into_bytes(), value.into_bytes())?;

        Ok(diff)
    }

    /// Remove all link indexes for a note (used when deleting or moving a note).
    pub fn remove_note_links<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        note_path: &str,
    ) -> Result<()> {
        let targets = Self::get_forward_links(engine, cf, note_path)?;

        // Remove this note from each target's link index
        for target in &targets {
            Self::remove_from_link_index(engine, cf, target, note_path)?;
        }

        // Delete the backlink index for this note
        let backlink_key = format!("{}{}", BACKLINK_PREFIX, note_path);
        engine.delete_cf(cf, backlink_key.into_bytes())?;

        Ok(())
    }

    /// Get all notes that link TO the given note (backlinks).
    pub fn get_backlinks<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        note_path: &str,
    ) -> Result<Vec<String>> {
        let key = format!("{}{}", LINK_PREFIX, note_path);
        match engine.get_cf(cf, key.into_bytes())? {
            Some(bytes) => {
                let value = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&value)
                    .map_err(|e| LsmError::InvalidArgument(format!("JSON parse error: {}", e)))
            }
            None => Ok(Vec::new()),
        }
    }

    /// Get all notes that the given note links TO (forward links).
    pub fn get_forward_links<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        note_path: &str,
    ) -> Result<Vec<String>> {
        let key = format!("{}{}", BACKLINK_PREFIX, note_path);
        match engine.get_cf(cf, key.into_bytes())? {
            Some(bytes) => {
                let value = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&value)
                    .map_err(|e| LsmError::InvalidArgument(format!("JSON parse error: {}", e)))
            }
            None => Ok(Vec::new()),
        }
    }

    /// Rename a note and update all indexes accordingly.
    ///
    /// This is a higher-level operation that:
    /// 1. Removes all link indexes for `old_path`
    /// 2. Re-indexes links for `new_path`
    /// 3. Updates all notes that linked to `old_path` to now point to `new_path`
    pub fn rename_note<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        old_path: &str,
        new_path: &str,
        new_content: &[String],
    ) -> Result<()> {
        // Get all notes that linked to the old path
        let backlinks = Self::get_backlinks(engine, cf, old_path)?;

        // Remove old indexes
        Self::remove_note_links(engine, cf, old_path)?;

        // Update all notes that pointed to old_path -> now point to new_path
        for source in &backlinks {
            // Remove old_path from this source's link index
            Self::remove_from_link_index(engine, cf, old_path, source)?;
            // Add new_path to this source's link index
            Self::add_to_link_index(engine, cf, new_path, source)?;
        }

        // Delete the old backlink entry for new_path (if it exists from a previous index)
        let new_backlink_key = format!("{}{}", BACKLINK_PREFIX, new_path);
        let _ = engine.delete_cf(cf, new_backlink_key.into_bytes());

        // Index the new note links
        Self::index_links(engine, cf, new_path, new_content)?;

        Ok(())
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Add a source note to the link index for a target.
    fn add_to_link_index<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        target: &str,
        source: &str,
    ) -> Result<()> {
        let key = format!("{}{}", LINK_PREFIX, target);
        let mut sources: Vec<String> = match engine.get_cf(cf, key.as_bytes())? {
            Some(bytes) => {
                let val = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&val).unwrap_or_default()
            }
            None => Vec::new(),
        };

        if !sources.contains(&source.to_string()) {
            sources.push(source.to_string());
            let value = serde_json::to_string(&sources)
                .map_err(|e| LsmError::InvalidArgument(format!("JSON error: {}", e)))?;
            engine.put_cf(cf, key.into_bytes(), value.into_bytes())?;
        }

        Ok(())
    }

    /// Remove a source note from the link index for a target.
    fn remove_from_link_index<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        target: &str,
        source: &str,
    ) -> Result<()> {
        let key = format!("{}{}", LINK_PREFIX, target);
        let mut sources: Vec<String> = match engine.get_cf(cf, key.as_bytes())? {
            Some(bytes) => {
                let val = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&val).unwrap_or_default()
            }
            None => return Ok(()),
        };

        sources.retain(|s| s != source);

        if sources.is_empty() {
            engine.delete_cf(cf, key.into_bytes())?;
        } else {
            let value = serde_json::to_string(&sources)
                .map_err(|e| LsmError::InvalidArgument(format!("JSON error: {}", e)))?;
            engine.put_cf(cf, key.into_bytes(), value.into_bytes())?;
        }

        Ok(())
    }

    /// Compute the diff between old and new link targets.
    fn compute_link_diff(old: &[String], new: &[String]) -> LinkDiff {
        let added: Vec<String> = new.iter().filter(|t| !old.contains(t)).cloned().collect();

        let removed: Vec<String> = old.iter().filter(|t| !new.contains(t)).cloned().collect();

        LinkDiff { added, removed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::storage::cache::GlobalBlockCache;
    use std::sync::Arc;

    fn create_test_engine() -> crate::core::engine::Engine<Arc<GlobalBlockCache>> {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        crate::core::engine::Engine::new_from_config(&config, GlobalBlockCache::new(10, 4096))
            .unwrap()
    }

    #[test]
    fn test_index_links_empty() {
        let engine = create_test_engine();
        let new_targets: Vec<String> = vec!["note-a".to_string(), "note-b".to_string()];

        let diff = NoteIndex::index_links(&engine, "default", "source-note", &new_targets).unwrap();

        assert_eq!(diff.added.len(), 2);
        assert!(diff.removed.is_empty());

        // Verify backlinks from target perspective
        let backlinks = NoteIndex::get_backlinks(&engine, "default", "note-a").unwrap();
        assert_eq!(backlinks, vec!["source-note"]);

        // Verify forward links from source perspective
        let forward = NoteIndex::get_forward_links(&engine, "default", "source-note").unwrap();
        assert_eq!(forward, vec!["note-a", "note-b"]);
    }

    #[test]
    fn test_index_links_update() {
        let engine = create_test_engine();

        // First index: links to A and B
        let targets1 = vec!["note-a".to_string(), "note-b".to_string()];
        NoteIndex::index_links(&engine, "default", "source", &targets1).unwrap();

        // Second index: links to B and C (A removed, C added)
        let targets2 = vec!["note-b".to_string(), "note-c".to_string()];
        let diff = NoteIndex::index_links(&engine, "default", "source", &targets2).unwrap();

        assert_eq!(diff.added, vec!["note-c"]);
        assert_eq!(diff.removed, vec!["note-a"]);

        // A should no longer have source as backlink
        let backlinks_a = NoteIndex::get_backlinks(&engine, "default", "note-a").unwrap();
        assert!(backlinks_a.is_empty());

        // B should still have source
        let backlinks_b = NoteIndex::get_backlinks(&engine, "default", "note-b").unwrap();
        assert_eq!(backlinks_b, vec!["source"]);

        // C should now have source
        let backlinks_c = NoteIndex::get_backlinks(&engine, "default", "note-c").unwrap();
        assert_eq!(backlinks_c, vec!["source"]);
    }

    #[test]
    fn test_remove_note_links() {
        let engine = create_test_engine();

        let targets = vec!["note-a".to_string(), "note-b".to_string()];
        NoteIndex::index_links(&engine, "default", "source", &targets).unwrap();

        NoteIndex::remove_note_links(&engine, "default", "source").unwrap();

        // Backlinks should be cleaned up
        let backlinks_a = NoteIndex::get_backlinks(&engine, "default", "note-a").unwrap();
        assert!(backlinks_a.is_empty());

        let forward = NoteIndex::get_forward_links(&engine, "default", "source").unwrap();
        assert!(forward.is_empty());
    }

    #[test]
    fn test_rename_note() {
        let engine = create_test_engine();

        // Note-X links to note-A and note-B
        let targets_x = vec!["note-a".to_string(), "note-b".to_string()];
        NoteIndex::index_links(&engine, "default", "note-x", &targets_x).unwrap();

        // Note-Y links to note-X
        let targets_y = vec!["note-x".to_string()];
        NoteIndex::index_links(&engine, "default", "note-y", &targets_y).unwrap();

        // Rename note-X to note-X-renamed
        let new_content = vec!["note-a".to_string(), "note-c".to_string()];
        NoteIndex::rename_note(&engine, "default", "note-x", "note-x-renamed", &new_content)
            .unwrap();

        // note-y should now link to note-x-renamed
        let backlinks = NoteIndex::get_backlinks(&engine, "default", "note-x-renamed").unwrap();
        assert!(backlinks.contains(&"note-y".to_string()));

        // Old note-x should have no backlinks
        let old_backlinks = NoteIndex::get_backlinks(&engine, "default", "note-x").unwrap();
        assert!(old_backlinks.is_empty());
    }

    #[test]
    fn test_self_link() {
        let engine = create_test_engine();

        // A note linking to itself should work without infinite loops
        let targets = vec!["note-a".to_string(), "self".to_string()];
        NoteIndex::index_links(&engine, "default", "note-a", &targets).unwrap();

        let backlinks = NoteIndex::get_backlinks(&engine, "default", "self").unwrap();
        assert!(backlinks.contains(&"note-a".to_string()));
    }

    #[test]
    fn test_circular_links() {
        let engine = create_test_engine();

        let targets_a = vec!["note-b".to_string()];
        NoteIndex::index_links(&engine, "default", "note-a", &targets_a).unwrap();

        let targets_b = vec!["note-c".to_string()];
        NoteIndex::index_links(&engine, "default", "note-b", &targets_b).unwrap();

        let targets_c = vec!["note-a".to_string()];
        NoteIndex::index_links(&engine, "default", "note-c", &targets_c).unwrap();

        // Circular: A→B→C→A — should not stack overflow or panic
        let backlinks_a = NoteIndex::get_backlinks(&engine, "default", "note-a").unwrap();
        assert_eq!(backlinks_a, vec!["note-c"]);
    }

    #[test]
    fn test_empty_targets() {
        let engine = create_test_engine();

        let diff = NoteIndex::index_links(&engine, "default", "empty-note", &[]).unwrap();
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());

        let forward = NoteIndex::get_forward_links(&engine, "default", "empty-note").unwrap();
        assert!(forward.is_empty());
    }

    #[test]
    fn test_noop_update() {
        let engine = create_test_engine();

        let targets = vec!["a".to_string(), "b".to_string()];
        NoteIndex::index_links(&engine, "default", "note", &targets).unwrap();

        // Same targets — should be a no-op
        let diff = NoteIndex::index_links(&engine, "default", "note", &targets).unwrap();
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
    }
}
