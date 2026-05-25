//! Wikilink, tag, and YAML frontmatter parser for Obsidian-compatible notes.
//!
//! # Syntax support
//!
//! - `[[Note Name]]` — basic wikilink
//! - `[[Note Name|Display Text]]` — wikilink with alias
//! - `[[#heading]]` — link to heading in same note
//! - `[[Note Name#heading]]` — link to heading in another note
//! - `[[Note Name|Display#heading]]` — combined
//! - `[[Note Name#^block-id]]` — block reference
//! - `![[image.png]]` — embedded file / embed
//! - `#tag` — inline tag
//! - `#tag/subtag` — nested tag

use std::collections::HashMap;

/// The type of a parsed link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkType {
    /// `[[Note Name]]` or `[[Note Name|Display]]`
    WikiLink,
    /// `![[file.png]]` — embedded file
    Embed,
    /// `[[#heading]]` — heading reference within same note
    HeadingRef,
    /// `[[Note Name#^block-id]]` — block reference
    BlockRef,
}

/// A single parsed wikilink with its position in the source content.
#[derive(Debug, Clone)]
pub struct Wikilink {
    /// The target note path, heading, or block ID.
    pub target: String,
    /// Optional display text (e.g., `[[target|display]]`).
    pub display_text: Option<String>,
    /// The type of link.
    pub link_type: LinkType,
    /// Byte offset where the link starts in the original content.
    pub start_offset: usize,
    /// Byte offset where the link ends in the original content.
    pub end_offset: usize,
}

/// Parsed YAML frontmatter from a note.
#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
    /// Note title from `title:` field.
    pub title: Option<String>,
    /// Alternative names from `aliases:` field.
    pub aliases: Vec<String>,
    /// Tags from `tags:` field.
    pub tags: Vec<String>,
    /// Creation date from `created:` field.
    pub created: Option<String>,
    /// Last update date from `updated:` field.
    pub updated: Option<String>,
    /// Any other custom frontmatter fields.
    pub custom: HashMap<String, String>,
}

/// The result of parsing a note's content.
#[derive(Debug, Clone)]
pub struct ParsedNote {
    /// The raw markdown content (after stripping frontmatter).
    pub content: String,
    /// Extracted wikilinks.
    pub links: Vec<Wikilink>,
    /// Extracted inline tags (from `#tag` syntax in content).
    pub inline_tags: Vec<String>,
    /// Parsed frontmatter (YAML between `---` markers).
    pub frontmatter: Frontmatter,
}

/// Extract all `[[wikilinks]]` from markdown content.
///
/// Supports:
/// - `[[target]]`
/// - `[[target|display]]`
/// - `[[#heading]]`
/// - `[[target#heading]]`
/// - `[[target#^block-id]]`
/// - `![[embed]]`
pub fn parse_wikilinks(content: &str) -> Vec<Wikilink> {
    let mut links = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for `![[` (embed) or `[[` (link)
        if i + 1 < len {
            let is_embed = bytes[i] == b'!' && i + 2 < len && bytes[i + 1] == b'[' && bytes[i + 2] == b'[';
            let is_link = bytes[i] == b'[' && i + 1 < len && bytes[i + 1] == b'[';

            if is_embed || is_link {
                let start = i;
                let content_start = if is_embed { i + 3 } else { i + 2 };

                // Find the closing `]]`
                if let Some(end) = find_closing_brackets(bytes, content_start) {
                    let inner = &content[content_start..end];
                    let link_type = if is_embed {
                        LinkType::Embed
                    } else if inner.starts_with('#') {
                        if inner.contains('^') {
                            LinkType::BlockRef
                        } else {
                            LinkType::HeadingRef
                        }
                    } else if inner.contains('#') {
                        if inner.contains("^block") || inner.contains('^') {
                            LinkType::BlockRef
                        } else {
                            LinkType::WikiLink
                        }
                    } else {
                        LinkType::WikiLink
                    };

                    let (target, display_text) = if let Some(pipe_pos) = inner.find('|') {
                        let target = inner[..pipe_pos].trim().to_string();
                        let display = inner[pipe_pos + 1..].trim().to_string();
                        (target, if display.is_empty() { None } else { Some(display) })
                    } else {
                        (inner.trim().to_string(), None)
                    };

                    links.push(Wikilink {
                        target,
                        display_text,
                        link_type,
                        start_offset: start,
                        end_offset: end + 2,
                    });

                    i = end + 2;
                    continue;
                }
            }
        }
        i += 1;
    }

    links
}

/// Find the closing `]]` for a wikilink starting at `start`.
/// Returns the index of the first `]` in the closing `]]`.
fn find_closing_brackets(bytes: &[u8], mut start: usize) -> Option<usize> {
    while start + 1 < bytes.len() {
        if bytes[start] == b']' && bytes[start + 1] == b']' {
            return Some(start);
        }
        start += 1;
    }
    None
}

/// Extract all `#tags` from markdown content.
///
/// Rules:
/// - Tag must start with `#` at a word boundary
/// - Allowed chars: `[a-zA-Z0-9_/-]`
/// - Tags inside code blocks, inline code, and HTML comments are ignored
/// - Nested tags (`#tag/subtag`) are stored as full path
/// - Max tag length: 100 chars
pub fn parse_tags(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut in_code_block = false;
    let mut in_inline_code = false;
    let mut in_html_comment = false;
    let mut i = 0;
    let bytes = content.as_bytes();
    let len = bytes.len();

    while i < len {
        // Track code blocks (```)
        if i + 2 < len && &bytes[i..i+3] == b"```" {
            in_code_block = !in_code_block;
            i += 3;
            continue;
        }

        // Track inline code (`)
        if bytes[i] == b'`' && !in_code_block {
            in_inline_code = !in_inline_code;
            i += 1;
            continue;
        }

        // Track HTML comments (<!-- -->)
        if i + 3 < len && &bytes[i..i+4] == b"<!--" {
            in_html_comment = true;
            i += 4;
            continue;
        }
        if in_html_comment && i + 2 < len && &bytes[i..i+3] == b"-->" {
            in_html_comment = false;
            i += 3;
            continue;
        }

        if !in_code_block && !in_inline_code && !in_html_comment && bytes[i] == b'#' {
            // Check it's at a word boundary (not preceded by alphanumeric)
            if i == 0 || is_tag_boundary(bytes[i - 1]) {
                let tag_start = i + 1;
                let mut tag_end = tag_start;
                while tag_end < len && is_tag_char(bytes[tag_end]) {
                    tag_end += 1;
                }
                if tag_end > tag_start && (tag_end - tag_start) <= 100 {
                    let tag = content[tag_start..tag_end].to_string();
                    if !tags.contains(&tag) {
                        tags.push(tag);
                    }
                }
                i = tag_end;
                continue;
            }
        }

        i += 1;
    }

    tags
}

/// Returns true if the byte is a valid tag boundary (not alphanumeric).
fn is_tag_boundary(b: u8) -> bool {
    !b.is_ascii_alphanumeric() && b != b'_'
}

/// Returns true if the byte is a valid tag character.
fn is_tag_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'/' || b == b'-'
}

/// Parse YAML frontmatter between `---` markers.
///
/// Returns the parsed frontmatter and the content after the frontmatter block.
pub fn parse_frontmatter(content: &str) -> (Frontmatter, &str) {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return (Frontmatter::default(), content);
    }

    // Find the closing `---`
    let after_first = &content[3..];
    if let Some(end) = after_first.find("\n---") {
        let yaml_block = &after_first[..end];
        let fm = parse_yaml_block(yaml_block);
        let rest = after_first[end + 4..].trim_start();
        (fm, rest)
    } else {
        (Frontmatter::default(), content)
    }
}

/// Parse a block of YAML key-value pairs.
fn parse_yaml_block(block: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut lines: Vec<&str> = Vec::new();

    // Collect all non-empty lines
    for line in block.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();

            if value.is_empty() || value.starts_with('[') || value.starts_with('-') {
                // Multi-line value (list)
                let mut list_values = Vec::new();

                if value.starts_with('[') && value.ends_with(']') {
                    // Parse `[item1, item2, ...]` inline list
                    let inner = value.trim_start_matches('[').trim_end_matches(']');
                    list_values = inner.split(',')
                        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                } else {
                    // Parse list items starting with `- ` on following lines
                    i += 1;
                    while i < lines.len() {
                        let next = lines[i];
                        if let Some(item) = next.strip_prefix("- ") {
                            list_values.push(item.trim().to_string());
                            i += 1;
                        } else if let Some(item) = next.strip_prefix('-') {
                            list_values.push(item.trim().to_string());
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }

                match key.as_str() {
                    "aliases" => fm.aliases = list_values,
                    "tags" => fm.tags = list_values,
                    _ => {
                        for v in list_values {
                            fm.custom.insert(format!("{}_item", key), v);
                        }
                    }
                }
            } else {
                match key.as_str() {
                    "title" => fm.title = Some(value.trim_matches('"').trim_matches('\'').to_string()),
                    "created" => fm.created = Some(value.to_string()),
                    "updated" => fm.updated = Some(value.to_string()),
                    "tags" => {
                        // Single tag value
                        let tag = value.trim_matches('"').trim_matches('\'').to_string();
                        if !tag.is_empty() {
                            fm.tags.push(tag);
                        }
                    }
                    _ => {
                        fm.custom.insert(key, value.trim_matches('"').trim_matches('\'').to_string());
                    }
                }
            }
        }
        i += 1;
    }

    fm
}

/// Parse a complete note: extract frontmatter, wikilinks, and tags.
pub fn parse_note(content: &str) -> ParsedNote {
    let (frontmatter, body) = parse_frontmatter(content);
    let links = parse_wikilinks(body);
    let inline_tags = parse_tags(body);

    // Merge frontmatter tags with inline tags (dedup)
    let mut all_tags = frontmatter.tags.clone();
    for tag in &inline_tags {
        if !all_tags.contains(tag) {
            all_tags.push(tag.clone());
        }
    }

    ParsedNote {
        content: body.to_string(),
        links,
        inline_tags: all_tags,
        frontmatter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Wikilink tests ──────────────────────────────────────────────────

    #[test]
    fn test_basic_wikilink() {
        let links = parse_wikilinks("Hello [[Note Name]] world");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "Note Name");
        assert_eq!(links[0].link_type, LinkType::WikiLink);
        assert_eq!(links[0].display_text, None);
    }

    #[test]
    fn test_wikilink_with_alias() {
        let links = parse_wikilinks("See [[target|display text]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "target");
        assert_eq!(links[0].display_text, Some("display text".to_string()));
    }

    #[test]
    fn test_wikilink_empty_alias() {
        let links = parse_wikilinks("See [[target|]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "target");
        assert_eq!(links[0].display_text, None);
    }

    #[test]
    fn test_embed_wikilink() {
        let links = parse_wikilinks("![[image.png]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "image.png");
        assert_eq!(links[0].link_type, LinkType::Embed);
    }

    #[test]
    fn test_heading_ref() {
        let links = parse_wikilinks("See [[#installation]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "#installation");
        assert_eq!(links[0].link_type, LinkType::HeadingRef);
    }

    #[test]
    fn test_note_with_heading_ref() {
        let links = parse_wikilinks("See [[Note Name#installation]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "Note Name#installation");
        assert_eq!(links[0].link_type, LinkType::WikiLink);
    }

    #[test]
    fn test_block_ref() {
        let links = parse_wikilinks("See [[Note Name#^block-id]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "Note Name#^block-id");
        assert_eq!(links[0].link_type, LinkType::BlockRef);
    }

    #[test]
    fn test_multiple_wikilinks() {
        let links = parse_wikilinks("[[A]] and [[B|b]] and [[C]]");
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target, "A");
        assert_eq!(links[1].target, "B");
        assert_eq!(links[2].target, "C");
    }

    #[test]
    fn test_no_wikilinks() {
        let links = parse_wikilinks("Just plain text");
        assert!(links.is_empty());
    }

    #[test]
    fn test_unclosed_bracket() {
        let links = parse_wikilinks("[[unclosed");
        assert!(links.is_empty());
    }

    #[test]
    fn test_wikilink_with_special_chars() {
        let links = parse_wikilinks("[[my-note_v2/readme]]");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "my-note_v2/readme");
    }

    #[test]
    fn test_wikilink_positions() {
        let links = parse_wikilinks("prefix [[link]] suffix");
        assert_eq!(links.len(), 1);
        assert_eq!(&links[0].target, "link");
        // The link starts at position 7 (0-indexed, after "prefix ")
        assert_eq!(links[0].start_offset, 7);
        assert_eq!(links[0].end_offset, 15);
    }

    #[test]
    fn test_wikilink_alias_with_pipe() {
        let links = parse_wikilinks("[[note|display with spaces]]");
        assert_eq!(links[0].target, "note");
        assert_eq!(links[0].display_text, Some("display with spaces".to_string()));
    }

    // ── Tag tests ───────────────────────────────────────────────────────

    #[test]
    fn test_basic_tag() {
        let tags = parse_tags("This is a #tag in text");
        assert_eq!(tags, vec!["tag"]);
    }

    #[test]
    fn test_multiple_tags() {
        let tags = parse_tags("#rust and #web and #database");
        assert_eq!(tags, vec!["rust", "web", "database"]);
    }

    #[test]
    fn test_nested_tag() {
        let tags = parse_tags("Topic #tech/rust/async");
        assert_eq!(tags, vec!["tech/rust/async"]);
    }

    #[test]
    fn test_tag_with_hyphen() {
        let tags = parse_tags("Tagged #my-tag_name");
        assert_eq!(tags, vec!["my-tag_name"]);
    }

    #[test]
    fn test_tag_inside_code_block() {
        let tags = parse_tags("Text\n```\n#tag_inside_code\n```\nMore text");
        let empty: Vec<String> = Vec::new();
        assert_eq!(tags, empty);
    }

    #[test]
    fn test_tag_after_hash_in_url() {
        // `#` in URL fragment should not be parsed as tag
        // The `#section` in `page#section` is preceded by alphanumeric `e`,
        // so the tag boundary check correctly rejects it.
        let tags = parse_tags("Check https://example.com/page#section");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_no_tags() {
        let tags = parse_tags("Plain text with no hash symbols");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tag_dedup() {
        let tags = parse_tags("#rust #rust #web");
        assert_eq!(tags, vec!["rust", "web"]);
    }

    #[test]
    fn test_tag_length_limit() {
        let long_tag = format!("#{}", "a".repeat(150));
        let tags = parse_tags(&long_tag);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tag_in_inline_code() {
        let tags = parse_tags("Text `#tag_inside_code` more text");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tag_in_html_comment() {
        let tags = parse_tags("Text <!-- #tag_in_comment --> more text");
        assert!(tags.is_empty());
    }

    // ── Frontmatter tests ───────────────────────────────────────────────

    #[test]
    fn test_basic_frontmatter() {
        let content = "---\ntitle: My Note\ncreated: 2026-05-25\n---\n\nNote content here";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.title, Some("My Note".to_string()));
        assert_eq!(fm.created, Some("2026-05-25".to_string()));
        assert_eq!(body, "Note content here");
    }

    #[test]
    fn test_no_frontmatter() {
        let (fm, body) = parse_frontmatter("Just content");
        assert_eq!(fm.title, None);
        assert_eq!(body, "Just content");
    }

    #[test]
    fn test_frontmatter_with_tags_list() {
        let content = "---\ntitle: Project\ntags: [rust, web, database]\n---\n\nBody";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.title, Some("Project".to_string()));
        assert_eq!(fm.tags, vec!["rust", "web", "database"]);
        assert_eq!(body, "Body");
    }

    #[test]
    fn test_frontmatter_with_aliases() {
        let content = "---\ntitle: My Note\naliases: [MN, My-Note]\n---\n\nBody";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.aliases, vec!["MN", "My-Note"]);
        assert_eq!(body, "Body");
    }

    #[test]
    fn test_frontmatter_with_custom_fields() {
        let content = "---\ntitle: Note\nstatus: draft\nauthor: Alice\n---\n\nBody";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.custom.get("status"), Some(&"draft".to_string()));
        assert_eq!(fm.custom.get("author"), Some(&"Alice".to_string()));
        assert_eq!(body, "Body");
    }

    #[test]
    fn test_frontmatter_with_yaml_list() {
        let content = "---\ntags:\n  - rust\n  - web\n  - database\n---\n\nBody";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.tags, vec!["rust", "web", "database"]);
        assert_eq!(body, "Body");
    }

    #[test]
    fn test_frontmatter_empty() {
        let content = "---\n---\n\nBody";
        let (fm, body) = parse_frontmatter(content);
        assert_eq!(fm.title, None);
        assert_eq!(body, "Body");
    }

    // ── Full note parse tests ───────────────────────────────────────────

    #[test]
    fn test_parse_full_note() {
        let content = "---\ntitle: My Project\ncreated: 2026-05-25\ntags: [rust, web]\n---\n\n# My Project\n\nThis is about [[feature-x]] and [[feature-y|Y Feature]].\n\nSome #important notes here.";
        let parsed = parse_note(content);

        assert_eq!(parsed.frontmatter.title, Some("My Project".to_string()));
        assert_eq!(parsed.content, "# My Project\n\nThis is about [[feature-x]] and [[feature-y|Y Feature]].\n\nSome #important notes here.");

        // 2 wikilinks
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(parsed.links[0].target, "feature-x");
        assert_eq!(parsed.links[1].target, "feature-y");

        // Tags from frontmatter + inline, deduped
        assert!(parsed.inline_tags.contains(&"rust".to_string()));
        assert!(parsed.inline_tags.contains(&"web".to_string()));
        assert!(parsed.inline_tags.contains(&"important".to_string()));
    }

    #[test]
    fn test_parse_note_no_frontmatter() {
        let content = "Just a simple note with [[a link]] and #tag";
        let parsed = parse_note(content);
        assert_eq!(parsed.frontmatter.title, None);
        assert_eq!(parsed.links.len(), 1);
        assert_eq!(parsed.inline_tags, vec!["tag"]);
    }
}
