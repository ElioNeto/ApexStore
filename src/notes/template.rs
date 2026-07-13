//! Template engine for daily notes with `{{variable}}` and `{{date:format}}` syntax.
//!
//! # Template Syntax
//!
//! - `{{variable}}` — Substituted with a value from the variables map.
//! - `{{date:format}}` — Current date formatted with chrono (e.g., `{{date:%Y-%m-%d}}`).
//! - `{{time:format}}` — Current time formatted with chrono (e.g., `{{time:%H:%M:%S}}`).
//!
//! # Storage
//!
//! Templates are stored in the LSM engine under the `default` column family
//! with key prefix `__template:`.

use crate::infra::error::{LsmError, Result};
use crate::notes::NoteEngine;
use crate::storage::cache::Cache;
use chrono::Local;
use std::collections::HashMap;

/// A note template with name, content, and declared variables.
#[derive(Debug, Clone)]
pub struct NoteTemplate {
    /// Template name (used as the storage key).
    pub name: String,
    /// Template body content with `{{...}}` placeholders.
    pub content: String,
    /// Variable names extracted from the template content.
    pub variables: Vec<String>,
}

/// Render a template string by substituting `{{variable}}`, `{{date:format}}`,
/// and `{{time:format}}` placeholders.
///
/// # Errors
///
/// Returns `InvalidArgument` if a required variable is missing from the map.
pub fn render_template(template: &str, variables: &HashMap<String, String>) -> Result<String> {
    let mut result = String::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        // Push everything before the placeholder
        result.push_str(&rest[..start]);

        // Find the closing `}}`
        let after_start = &rest[start + 2..];
        let end = after_start
            .find("}}")
            .ok_or_else(|| LsmError::InvalidArgument("Unclosed template placeholder".into()))?;

        let placeholder = &after_start[..end];

        // Determine placeholder type
        if let Some(date_fmt) = placeholder.strip_prefix("date:") {
            let now = Local::now();
            result.push_str(&now.format(date_fmt).to_string());
        } else if let Some(time_fmt) = placeholder.strip_prefix("time:") {
            let now = Local::now();
            result.push_str(&now.format(time_fmt).to_string());
        } else {
            // Regular variable substitution
            let var_name = placeholder.trim();
            match variables.get(var_name) {
                Some(val) => result.push_str(val),
                None => {
                    return Err(LsmError::InvalidArgument(format!(
                        "Missing template variable: {}",
                        var_name
                    )));
                }
            }
        }

        rest = &after_start[end + 2..];
    }

    // Push remaining text after last placeholder
    result.push_str(rest);

    Ok(result)
}

/// Extract variable names (non-date, non-time placeholders) from a template string.
#[cfg(test)]
fn extract_variables(template: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            let placeholder = &after_start[..end];
            // Only collect non-date, non-time variables
            if !placeholder.starts_with("date:") && !placeholder.starts_with("time:") {
                let var_name = placeholder.trim().to_string();
                if !vars.contains(&var_name) {
                    vars.push(var_name);
                }
            }
            rest = &after_start[end + 2..];
        } else {
            break;
        }
    }

    vars
}

/// List all saved template names.
pub fn list_templates<C: Cache>(engine: &NoteEngine<C>) -> Result<Vec<String>> {
    let prefix = "__template:";
    let (results, _cursor) = engine
        .engine()
        .search_prefix(prefix, None, crate::core::engine::MAX_SCAN_LIMIT)
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to list templates: {}", e)))?;

    let names: Vec<String> = results
        .into_iter()
        .filter_map(|(k, _v)| {
            let key = String::from_utf8_lossy(&k).to_string();
            key.strip_prefix("__template:").map(|s| s.to_string())
        })
        .collect();

    Ok(names)
}

/// Save a template to the engine.
///
/// The template content is stored under `__template:{name}` in the default CF.
pub fn save_template<C: Cache>(engine: &NoteEngine<C>, name: &str, content: &str) -> Result<()> {
    let key = format!("__template:{}", name);
    engine
        .engine()
        .put_cf("default", key.into_bytes(), content.as_bytes().to_vec())
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to save template: {}", e)))?;
    Ok(())
}

/// Delete a template from the engine.
pub fn delete_template<C: Cache>(engine: &NoteEngine<C>, name: &str) -> Result<()> {
    let key = format!("__template:{}", name);
    engine
        .engine()
        .delete_cf("default", key.into_bytes())
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to delete template: {}", e)))?;
    Ok(())
}

/// Get a template's content.
pub fn get_template<C: Cache>(engine: &NoteEngine<C>, name: &str) -> Result<Option<String>> {
    let key = format!("__template:{}", name);
    match engine.engine().get_cf("default", key.as_bytes()) {
        Ok(Some(bytes)) => Ok(Some(String::from_utf8_lossy(&bytes).to_string())),
        Ok(None) => Ok(None),
        Err(e) => Err(LsmError::InvalidArgument(format!(
            "Failed to get template: {}",
            e
        ))),
    }
}

/// Create a daily note from an optional template.
///
/// The note path follows the pattern `daily/YYYY-MM-DD`.
/// If a `template_name` is provided, the template is loaded, rendered with
/// `{{date:%Y-%m-%d}}` and `{{time:%H:%M:%S}}` variables, and saved.
/// If no template is given, an empty daily note is created.
pub fn create_daily_note<C: Cache>(
    engine: &NoteEngine<C>,
    template_name: Option<&str>,
) -> Result<String> {
    let now = Local::now();
    let date_str = now.format("%Y-%m-%d").to_string();
    let note_path = format!("daily/{}", date_str);

    // Check if already exists
    if let Ok(Some(_)) = engine.get_note(&note_path) {
        return Ok(note_path); // Already exists, return path
    }

    let content = match template_name {
        Some(tname) => {
            let raw = get_template(engine, tname)?.ok_or_else(|| {
                LsmError::InvalidArgument(format!("Template not found: {}", tname))
            })?;

            let mut variables = HashMap::new();
            // Pre-populate date/time variables
            variables.insert("date".to_string(), now.format("%Y-%m-%d").to_string());
            variables.insert("time".to_string(), now.format("%H:%M:%S").to_string());

            render_template(&raw, &variables)?
        }
        None => format!("# Daily Note — {}\n\n", date_str),
    };

    engine.put_note(&note_path, &content)?;
    Ok(note_path)
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
    fn test_render_template_simple() {
        let mut vars = HashMap::new();
        vars.insert("title".to_string(), "My Note".to_string());
        vars.insert("author".to_string(), "Alice".to_string());

        let result = render_template("# {{title}} by {{author}}", &vars).unwrap();
        assert_eq!(result, "# My Note by Alice");
    }

    #[test]
    fn test_render_template_date_format() {
        let vars = HashMap::new();
        let result = render_template("Date: {{date:%Y-%m-%d}}", &vars).unwrap();
        // Should contain today's date
        let today = Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(result, format!("Date: {}", today));
    }

    #[test]
    fn test_render_template_time_format() {
        let vars = HashMap::new();
        let result = render_template("Time: {{time:%H:%M:%S}}", &vars).unwrap();
        assert!(result.starts_with("Time: "));
        assert_eq!(result.len(), 14); // "Time: HH:MM:SS"
    }

    #[test]
    fn test_render_template_missing_variable() {
        let vars = HashMap::new();
        let result = render_template("Hello {{name}}", &vars);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Missing template variable"));
    }

    #[test]
    fn test_render_template_unclosed() {
        let vars = HashMap::new();
        let result = render_template("Hello {{name", &vars);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unclosed template placeholder"));
    }

    #[test]
    fn test_render_template_no_placeholders() {
        let vars = HashMap::new();
        let result = render_template("Plain text", &vars).unwrap();
        assert_eq!(result, "Plain text");
    }

    #[test]
    fn test_render_template_multiple() {
        let mut vars = HashMap::new();
        vars.insert("a".to_string(), "1".to_string());
        vars.insert("b".to_string(), "2".to_string());

        let result = render_template("{{a}} + {{b}} = {{a}}{{b}}", &vars).unwrap();
        assert_eq!(result, "1 + 2 = 12");
    }

    #[test]
    fn test_extract_variables() {
        let template = "Title: {{title}}\nAuthor: {{author}}\nDate: {{date:%Y-%m-%d}}";
        let vars = extract_variables(template);
        assert!(vars.contains(&"title".to_string()));
        assert!(vars.contains(&"author".to_string()));
        assert!(!vars.contains(&"date:%Y-%m-%d".to_string())); // date: prefixed excluded
    }

    #[test]
    fn test_save_and_list_templates() {
        let engine = create_note_engine();
        save_template(&engine, "weekly-report", "# Week {{week_number}}").unwrap();
        save_template(&engine, "meeting-notes", "# Meeting: {{topic}}").unwrap();

        let templates = list_templates(&engine).unwrap();
        assert!(templates.contains(&"weekly-report".to_string()));
        assert!(templates.contains(&"meeting-notes".to_string()));
    }

    #[test]
    fn test_get_template() {
        let engine = create_note_engine();
        save_template(&engine, "test", "Content: {{var}}").unwrap();

        let content = get_template(&engine, "test").unwrap();
        assert_eq!(content, Some("Content: {{var}}".to_string()));

        let missing = get_template(&engine, "nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_delete_template() {
        let engine = create_note_engine();
        save_template(&engine, "to-delete", "content").unwrap();
        delete_template(&engine, "to-delete").unwrap();

        let content = get_template(&engine, "to-delete").unwrap();
        assert!(content.is_none());
    }

    #[test]
    fn test_create_daily_note_default() {
        let engine = create_note_engine();
        let path = create_daily_note(&engine, None).unwrap();

        let today = Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(path, format!("daily/{}", today));

        let content = engine.get_note(&path).unwrap();
        assert!(content.is_some());
        assert!(content.unwrap().contains(&today));
    }

    #[test]
    fn test_create_daily_note_with_template() {
        let engine = create_note_engine();
        save_template(
            &engine,
            "daily",
            "# {{date:%Y-%m-%d}}\n\n## Tasks\n\n- [ ] ",
        )
        .unwrap();

        let path = create_daily_note(&engine, Some("daily")).unwrap();
        let content = engine.get_note(&path).unwrap().unwrap();
        assert!(content.contains("## Tasks"));
    }

    #[test]
    fn test_create_daily_note_idempotent() {
        let engine = create_note_engine();
        let path1 = create_daily_note(&engine, None).unwrap();
        let path2 = create_daily_note(&engine, None).unwrap();
        assert_eq!(path1, path2); // Same path returned
    }
}
