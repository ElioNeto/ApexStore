//! Note graph assembly — builds graph representations of note connections
//! for visualization in the frontend graph view.
//!
//! The graph is returned as a JSON-serializable structure compatible with
//! D3.js force layout and vis.js.

use crate::infra::error::Result;
use crate::storage::cache::Cache;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// A node in the note graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Unique identifier for the node (note path).
    pub id: String,
    /// Display label for the node.
    pub label: String,
    /// Optional grouping/tag for color-coding.
    pub group: Option<String>,
    /// Node size (proportional to connection count).
    pub size: usize,
}

/// An edge connecting two nodes in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source node ID.
    pub source: String,
    /// Target node ID.
    pub target: String,
    /// Edge weight (always 1 for wikilinks, can be higher for multiple links).
    pub weight: usize,
}

/// The complete graph structure, serializable to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphData {
    /// All nodes in the graph.
    pub nodes: Vec<GraphNode>,
    /// All edges connecting nodes.
    pub edges: Vec<GraphEdge>,
    /// The root/center node ID (the note being viewed).
    pub root: String,
}

/// Graph traversal depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GraphDepth {
    /// Only direct neighbors (1 hop).
    Direct = 1,
    /// Neighbors of neighbors (2 hops).
    Extended = 2,
    /// Maximum depth (3 hops).
    Deep = 3,
}

impl GraphDepth {
    pub fn from_usize(depth: usize) -> Self {
        match depth {
            0 | 1 => GraphDepth::Direct,
            2 => GraphDepth::Extended,
            _ => GraphDepth::Deep,
        }
    }
}

/// Configuration for graph assembly.
#[derive(Debug, Clone)]
pub struct GraphConfig {
    /// Maximum traversal depth.
    pub depth: GraphDepth,
    /// Whether to include tag-based grouping.
    pub include_tags: bool,
    /// Maximum number of nodes to include.
    pub max_nodes: usize,
    /// Optional tag filter — only include notes with this tag.
    pub tag_filter: Option<String>,
    /// Whether to include isolated nodes (notes with no connections).
    pub include_isolated: bool,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            depth: GraphDepth::Direct,
            include_tags: true,
            max_nodes: 500,
            tag_filter: None,
            include_isolated: false,
        }
    }
}

/// The note graph engine — assembles graph data from note indexes.
pub struct NoteGraph;

impl NoteGraph {
    /// Build a graph centered on a specific note.
    ///
    /// Traverses links and backlinks up to `config.depth` hops and returns
    /// a `GraphData` structure with nodes and edges.
    pub fn build_graph<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        center_note: &str,
        config: &GraphConfig,
    ) -> Result<GraphData> {
        let mut nodes: HashMap<String, GraphNode> = HashMap::new();
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();

        // Add the center node
        let center_id = center_note.to_string();
        let center_label = Self::note_label(center_note);
        nodes.insert(
            center_id.clone(),
            GraphNode {
                id: center_id.clone(),
                label: center_label,
                group: None,
                size: 1,
            },
        );
        visited.insert(center_id.clone());
        queue.push_back((center_id.clone(), 0));

        // BFS traversal
        while let Some((current_note, current_depth)) = queue.pop_front() {
            if current_depth >= config.depth as usize {
                continue;
            }

            // Get forward links (notes this note points TO)
            let forward_links =
                crate::notes::index::NoteIndex::get_forward_links(engine, cf, &current_note)?;
            for target in &forward_links {
                if nodes.len() >= config.max_nodes {
                    break;
                }

                // Add edge
                edges.push(GraphEdge {
                    source: current_note.clone(),
                    target: target.clone(),
                    weight: 1,
                });

                // Update node connection count
                nodes
                    .entry(current_note.clone())
                    .and_modify(|n| n.size = n.size.saturating_add(1));
                nodes
                    .entry(target.clone())
                    .and_modify(|n| n.size = n.size.saturating_add(1));

                if !visited.contains(target) {
                    visited.insert(target.clone());
                    let label = Self::note_label(target);
                    nodes.insert(
                        target.clone(),
                        GraphNode {
                            id: target.clone(),
                            label,
                            group: None,
                            size: 1,
                        },
                    );
                    if current_depth + 1 < config.depth as usize {
                        queue.push_back((target.clone(), current_depth + 1));
                    }
                }
            }

            // Get backlinks (notes that point TO this note)
            let backlinks =
                crate::notes::index::NoteIndex::get_backlinks(engine, cf, &current_note)?;
            for source in &backlinks {
                if nodes.len() >= config.max_nodes {
                    break;
                }

                // Add edge
                edges.push(GraphEdge {
                    source: source.clone(),
                    target: current_note.clone(),
                    weight: 1,
                });

                // Update node connection count
                nodes
                    .entry(current_note.clone())
                    .and_modify(|n| n.size = n.size.saturating_add(1));
                nodes
                    .entry(source.clone())
                    .and_modify(|n| n.size = n.size.saturating_add(1));

                if !visited.contains(source) {
                    visited.insert(source.clone());
                    let label = Self::note_label(source);
                    nodes.insert(
                        source.clone(),
                        GraphNode {
                            id: source.clone(),
                            label,
                            group: None,
                            size: 1,
                        },
                    );
                    if current_depth + 1 < config.depth as usize {
                        queue.push_back((source.clone(), current_depth + 1));
                    }
                }
            }
        }

        // Apply tag filter if specified
        let mut nodes = if let Some(ref filter_tag) = config.tag_filter {
            let filtered: HashMap<String, GraphNode> = nodes
                .into_iter()
                .filter(|(id, _)| {
                    // Check if note has the tag (simplified: tag prefix scan)
                    Self::note_has_tag(engine, cf, id, filter_tag).unwrap_or(false)
                })
                .collect();
            filtered
        } else {
            nodes
        };

        // Filter edges to only include nodes that exist
        edges.retain(|e| nodes.contains_key(&e.source) && nodes.contains_key(&e.target));

        // Deduplicate edges (same source+target may appear from both forward and backlink traversal)
        let mut seen_edges: HashSet<(String, String)> = HashSet::new();
        edges.retain(|e| {
            let key = if e.source < e.target {
                (e.source.clone(), e.target.clone())
            } else {
                (e.target.clone(), e.source.clone())
            };
            seen_edges.insert(key)
        });

        // Apply tag-based grouping if enabled
        if config.include_tags {
            Self::apply_tag_groups(engine, cf, &mut nodes)?;
        }

        let node_list: Vec<GraphNode> = nodes.into_values().collect();

        Ok(GraphData {
            nodes: node_list,
            edges,
            root: center_note.to_string(),
        })
    }

    /// Generate a human-readable label for a note path.
    fn note_label(path: &str) -> String {
        // Get the last component of the path (filename without extension)
        let name = path.split('/').next_back().unwrap_or(path);
        name.trim_end_matches(".md").replace(['-', '_'], " ")
    }

    /// Check if a note has a specific tag (uses prefix scan on tag index).
    fn note_has_tag<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        note_path: &str,
        tag: &str,
    ) -> Result<bool> {
        // Scan tag index for this note
        let tag_key = format!("tag:{}", tag);
        match engine.get_cf(cf, tag_key.into_bytes())? {
            Some(bytes) => {
                let value = String::from_utf8_lossy(&bytes);
                let notes: Vec<String> = serde_json::from_str(&value).unwrap_or_default();
                Ok(notes.contains(&note_path.to_string()))
            }
            None => Ok(false),
        }
    }

    /// Apply tag-based grouping to nodes.
    fn apply_tag_groups<C: Cache>(
        engine: &crate::core::engine::Engine<C>,
        cf: &str,
        nodes: &mut HashMap<String, GraphNode>,
    ) -> Result<()> {
        // For each node, find its primary tag (first tag in its tag index)
        for (id, node) in nodes.iter_mut() {
            // Try to find a tag for this note by scanning the tag store
            // We use a simple heuristic: check most common tags
            let common_tags = ["important", "project", "reference", "archive", "personal"];
            for tag in &common_tags {
                let tag_key = format!("tag:{}", tag);
                if let Ok(Some(bytes)) = engine.get_cf(cf, tag_key.into_bytes()) {
                    let value = String::from_utf8_lossy(&bytes);
                    let notes: Vec<String> = serde_json::from_str(&value).unwrap_or_default();
                    if notes.contains(id) {
                        node.group = Some(tag.to_string());
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::notes::index::NoteIndex;
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
    fn test_basic_graph() {
        let engine = create_test_engine();

        // Set up: A → B, A → C
        let targets_a = vec!["note-b".to_string(), "note-c".to_string()];
        NoteIndex::index_links(&engine, "default", "note-a", &targets_a).unwrap();

        let config = GraphConfig::default();
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();

        assert_eq!(graph.root, "note-a");
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);

        // Root should have size = connections + 1
        let root_node = graph.nodes.iter().find(|n| n.id == "note-a").unwrap();
        assert!(root_node.size >= 2);
    }

    #[test]
    fn test_depth_traversal() {
        let engine = create_test_engine();

        // A → B → C
        NoteIndex::index_links(&engine, "default", "note-a", &["note-b".to_string()]).unwrap();
        NoteIndex::index_links(&engine, "default", "note-b", &["note-c".to_string()]).unwrap();

        // Depth 1: only A and B
        let config = GraphConfig {
            depth: GraphDepth::Direct,
            ..Default::default()
        };
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes.iter().any(|n| n.id == "note-a"));
        assert!(graph.nodes.iter().any(|n| n.id == "note-b"));

        // Depth 2: A, B, and C
        let config = GraphConfig {
            depth: GraphDepth::Extended,
            ..Default::default()
        };
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert!(graph.nodes.iter().any(|n| n.id == "note-c"));
    }

    #[test]
    fn test_graph_with_backlinks() {
        let engine = create_test_engine();

        // X → Y, Z → Y (Y has 2 backlinks)
        NoteIndex::index_links(&engine, "default", "note-x", &["note-y".to_string()]).unwrap();
        NoteIndex::index_links(&engine, "default", "note-z", &["note-y".to_string()]).unwrap();

        let config = GraphConfig::default();
        let graph = NoteGraph::build_graph(&engine, "default", "note-y", &config).unwrap();

        // Y, X, Z should all be in the graph
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
    }

    #[test]
    fn test_graph_max_nodes() {
        let engine = create_test_engine();

        // Connect A to many notes
        let many_targets: Vec<String> = (0..20).map(|i| format!("note-{}", i)).collect();
        NoteIndex::index_links(&engine, "default", "note-a", &many_targets).unwrap();

        let config = GraphConfig {
            max_nodes: 10,
            ..Default::default()
        };
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();

        assert!(graph.nodes.len() <= 10);
    }

    #[test]
    fn test_graph_isolated_note() {
        let engine = create_test_engine();

        // Note with no links
        let config = GraphConfig::default();
        let graph = NoteGraph::build_graph(&engine, "default", "lonely-note", &config).unwrap();

        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, "lonely-note");
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn test_graph_label_formatting() {
        assert_eq!(NoteGraph::note_label("my-note.md"), "my note");
        assert_eq!(NoteGraph::note_label("path/to/feature-x.md"), "feature x");
        assert_eq!(NoteGraph::note_label("simple"), "simple");
    }

    #[test]
    fn test_graph_serialization() {
        let engine = create_test_engine();

        NoteIndex::index_links(&engine, "default", "note-a", &["note-b".to_string()]).unwrap();

        let config = GraphConfig::default();
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();

        // Must serialize to valid JSON
        let json = serde_json::to_string(&graph).unwrap();
        assert!(json.contains("nodes"));
        assert!(json.contains("edges"));
        assert!(json.contains("root"));
    }

    #[test]
    fn test_edge_dedup() {
        let engine = create_test_engine();

        // A → B (forward link)
        NoteIndex::index_links(&engine, "default", "note-a", &["note-b".to_string()]).unwrap();
        // B → A (backlink — creates the same edge A-B)
        NoteIndex::index_links(&engine, "default", "note-b", &["note-a".to_string()]).unwrap();

        let config = GraphConfig::default();
        let graph = NoteGraph::build_graph(&engine, "default", "note-a", &config).unwrap();

        // Should only have 1 edge (A-B) despite both forward and backlink
        assert_eq!(graph.edges.len(), 1);
    }
}
