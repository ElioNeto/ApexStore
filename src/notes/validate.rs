//! Frontmatter schema validation system.
//!
//! Provides a schema-based validator for YAML frontmatter in notes.
//! Schemas define required fields, field types, allowed tags, and maximum
//! tag count. Validation produces human-readable error messages.
//!
//! # Storage
//!
//! Schemas are stored in the LSM engine under the `default` column family
//! with key prefix `__frontmatter_schema:`. A default schema is registered
//! on application startup.

use crate::infra::error::{LsmError, Result};
use crate::notes::parser::Frontmatter;
use crate::notes::NoteEngine;
use crate::storage::cache::Cache;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The type of a frontmatter field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    /// Text string value.
    String,
    /// Numeric value (parsed as f64).
    Number,
    /// Date string value.
    Date,
    /// List of tags (`[tag1, tag2]` or YAML list).
    TagList,
    /// List of strings.
    StringList,
}

impl FieldType {
    /// Human-readable name for the field type.
    pub fn name(&self) -> &str {
        match self {
            FieldType::String => "string",
            FieldType::Number => "number",
            FieldType::Date => "date",
            FieldType::TagList => "tag list",
            FieldType::StringList => "string list",
        }
    }
}

/// A schema that defines validation rules for frontmatter fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontmatterSchema {
    /// Field names that must be present in the frontmatter.
    pub required_fields: Vec<String>,
    /// Expected types for specific fields (`field_name -> FieldType`).
    pub field_types: HashMap<String, FieldType>,
    /// If set, only these tag values are allowed in the `tags` field.
    pub allowed_tags: Option<Vec<String>>,
    /// Maximum number of tags allowed (default: 20).
    pub max_tags: usize,
}

impl Default for FrontmatterSchema {
    fn default() -> Self {
        Self {
            required_fields: Vec::new(),
            field_types: HashMap::new(),
            allowed_tags: None,
            max_tags: 20,
        }
    }
}

/// Validate frontmatter against a schema.
///
/// Returns a list of human-readable validation error messages.
/// An empty vec means validation passed.
///
/// # Errors
///
/// Returns `LsmError::InvalidArgument` for internal issues.
pub fn validate_frontmatter(fm: &Frontmatter, schema: &FrontmatterSchema) -> Result<Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

    // Check required fields
    for field in &schema.required_fields {
        match field.as_str() {
            "title" => {
                if fm.title.is_none() || fm.title.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    errors.push(format!("Required field '{}' is missing or empty", field));
                }
            }
            "created" | "updated" => {
                let val = if field == "created" {
                    &fm.created
                } else {
                    &fm.updated
                };
                if val.as_ref().is_none_or(|s| s.trim().is_empty()) {
                    errors.push(format!("Required field '{}' is missing or empty", field));
                }
            }
            "tags" => {
                if fm.tags.is_empty() {
                    errors.push(format!("Required field '{}' is missing or empty", field));
                }
            }
            "aliases" => {
                // aliases are optional even if required (empty list is allowed)
                // Only error if the field concept doesn't exist at all — we can't
                // distinguish from our flat struct, so skip aliases from required check.
            }
            other => {
                // Check custom fields
                if !fm.custom.contains_key(other) {
                    errors.push(format!("Required field '{}' is missing", other));
                }
            }
        }
    }

    // Validate field types
    for (field_name, expected_type) in &schema.field_types {
        let value = match field_name.as_str() {
            "title" => fm.title.as_deref(),
            "created" => fm.created.as_deref(),
            "updated" => fm.updated.as_deref(),
            _ => fm.custom.get(field_name).map(|s| s.as_str()),
        };

        if let Some(val) = value {
            if !validate_type(val, expected_type) {
                errors.push(format!(
                    "Field '{}' expected type '{}' but got value: '{}'",
                    field_name,
                    expected_type.name(),
                    val
                ));
            }
        }
    }

    // Validate tags
    if let Some(allowed) = &schema.allowed_tags {
        for tag in &fm.tags {
            if !allowed.contains(tag) {
                errors.push(format!("Tag '{}' is not in the allowed list", tag));
            }
        }
    }

    if fm.tags.len() > schema.max_tags {
        errors.push(format!(
            "Too many tags: {} (max: {})",
            fm.tags.len(),
            schema.max_tags
        ));
    }

    Ok(errors)
}

/// Check if a string value matches the expected field type.
fn validate_type(value: &str, field_type: &FieldType) -> bool {
    match field_type {
        FieldType::String => true, // Any string is valid
        FieldType::Number => value.parse::<f64>().is_ok(),
        FieldType::Date => {
            // Accept ISO 8601 dates: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS
            value.len() >= 10
                && value.chars().any(|c| c == '-')
                && value.chars().filter(|c| *c == '-').count() >= 2
        }
        FieldType::TagList => {
            // Tags are already parsed as Vec<String> — we validate each tag
            // is non-empty and doesn't contain spaces
            !value.is_empty() && !value.contains(' ')
        }
        FieldType::StringList => !value.is_empty(),
    }
}

/// Get the default schema used throughout the application.
///
/// By default:
/// - `title` is required (String)
/// - `created` is optional but typed as Date
/// - `tags` is typed as TagList with max 20 tags
pub fn get_default_schema() -> FrontmatterSchema {
    let mut field_types = HashMap::new();
    field_types.insert("title".to_string(), FieldType::String);
    field_types.insert("created".to_string(), FieldType::Date);
    field_types.insert("updated".to_string(), FieldType::Date);
    field_types.insert("tags".to_string(), FieldType::TagList);

    FrontmatterSchema {
        required_fields: vec!["title".to_string()],
        field_types,
        allowed_tags: None,
        max_tags: 20,
    }
}

/// Save a frontmatter schema under a given name.
pub fn save_schema<C: Cache>(
    engine: &NoteEngine<C>,
    name: &str,
    schema: &FrontmatterSchema,
) -> Result<()> {
    let key = format!("__frontmatter_schema:{}", name);
    let value = serde_json::to_vec(schema)
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to serialize schema: {}", e)))?;
    engine
        .engine()
        .put_cf("default", key.into_bytes(), value)
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to save schema: {}", e)))?;
    Ok(())
}

/// Load a frontmatter schema by name.
pub fn load_schema<C: Cache>(
    engine: &NoteEngine<C>,
    name: &str,
) -> Result<Option<FrontmatterSchema>> {
    let key = format!("__frontmatter_schema:{}", name);
    match engine.engine().get_cf("default", key.as_bytes()) {
        Ok(Some(bytes)) => {
            let schema: FrontmatterSchema = serde_json::from_slice(&bytes).map_err(|e| {
                LsmError::InvalidArgument(format!("Failed to deserialize schema: {}", e))
            })?;
            Ok(Some(schema))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(LsmError::InvalidArgument(format!(
            "Failed to load schema: {}",
            e
        ))),
    }
}

/// Delete a frontmatter schema.
pub fn delete_schema<C: Cache>(engine: &NoteEngine<C>, name: &str) -> Result<()> {
    let key = format!("__frontmatter_schema:{}", name);
    engine
        .engine()
        .delete_cf("default", key.into_bytes())
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to delete schema: {}", e)))?;
    Ok(())
}

/// List all saved schema names.
pub fn list_schemas<C: Cache>(engine: &NoteEngine<C>) -> Result<Vec<String>> {
    let prefix = "__frontmatter_schema:";
    let (results, _cursor) = engine
        .engine()
        .search_prefix(prefix, None, crate::core::engine::MAX_SCAN_LIMIT)
        .map_err(|e| LsmError::InvalidArgument(format!("Failed to list schemas: {}", e)))?;

    let names: Vec<String> = results
        .into_iter()
        .filter_map(|(k, _v)| {
            let key = String::from_utf8_lossy(&k).to_string();
            key.strip_prefix("__frontmatter_schema:")
                .map(|s| s.to_string())
        })
        .collect();

    Ok(names)
}

/// Register the default schema on application startup (idempotent).
pub fn register_default_schema<C: Cache>(engine: &NoteEngine<C>) -> Result<()> {
    let existing = load_schema(engine, "default")?;
    if existing.is_none() {
        let schema = get_default_schema();
        save_schema(engine, "default", &schema)?;
    }
    Ok(())
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

    fn make_frontmatter(
        title: Option<&str>,
        tags: Vec<&str>,
        custom: Vec<(&str, &str)>,
    ) -> Frontmatter {
        let mut custom_map = HashMap::new();
        for (k, v) in custom {
            custom_map.insert(k.to_string(), v.to_string());
        }
        Frontmatter {
            title: title.map(|s| s.to_string()),
            aliases: vec![],
            tags: tags.into_iter().map(|s| s.to_string()).collect(),
            created: None,
            updated: None,
            custom: custom_map,
        }
    }

    #[test]
    fn test_validate_required_title_present() {
        let fm = make_frontmatter(Some("My Note"), vec![], vec![]);
        let schema = get_default_schema();
        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_required_title_missing() {
        let fm = make_frontmatter(None, vec![], vec![]);
        let schema = get_default_schema();
        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("title"));
    }

    #[test]
    fn test_validate_required_custom_field() {
        let fm = make_frontmatter(Some("Note"), vec![], vec![]);
        let mut schema = get_default_schema();
        schema.required_fields.push("status".to_string());

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("status"));
    }

    #[test]
    fn test_validate_number_type() {
        let fm = make_frontmatter(Some("Note"), vec![], vec![("priority", "abc")]);
        let mut schema = get_default_schema();
        schema
            .field_types
            .insert("priority".to_string(), FieldType::Number);

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("priority"));
        assert!(errors[0].contains("number"));
    }

    #[test]
    fn test_validate_number_type_valid() {
        let fm = make_frontmatter(Some("Note"), vec![], vec![("priority", "42")]);
        let mut schema = get_default_schema();
        schema
            .field_types
            .insert("priority".to_string(), FieldType::Number);

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_allowed_tags() {
        let fm = make_frontmatter(Some("Note"), vec!["rust", "python"], vec![]);
        let mut schema = get_default_schema();
        schema.allowed_tags = Some(vec!["rust".to_string(), "web".to_string()]);

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("python"));
    }

    #[test]
    fn test_validate_allowed_tags_pass() {
        let fm = make_frontmatter(Some("Note"), vec!["rust", "web"], vec![]);
        let mut schema = get_default_schema();
        schema.allowed_tags = Some(vec!["rust".to_string(), "web".to_string()]);

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_max_tags() {
        let fm = make_frontmatter(Some("Note"), vec!["a", "b", "c"], vec![]);
        let mut schema = get_default_schema();
        schema.max_tags = 2;

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(!errors.is_empty());
        assert!(errors[0].contains("Too many tags"));
    }

    #[test]
    fn test_validate_pass_all_rules() {
        let fm = make_frontmatter(
            Some("Valid Note"),
            vec!["rust", "web"],
            vec![("priority", "1"), ("status", "draft")],
        );
        let mut schema = get_default_schema();
        schema.required_fields.push("status".to_string());
        schema
            .field_types
            .insert("priority".to_string(), FieldType::Number);
        schema.allowed_tags = Some(vec!["rust".to_string(), "web".to_string()]);
        schema.max_tags = 5;

        let errors = validate_frontmatter(&fm, &schema).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_save_and_load_schema() {
        let engine = create_note_engine();
        let schema = get_default_schema();
        save_schema(&engine, "test-schema", &schema).unwrap();

        let loaded = load_schema(&engine, "test-schema").unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().required_fields, vec!["title"]);
    }

    #[test]
    fn test_list_schemas() {
        let engine = create_note_engine();
        save_schema(&engine, "schema-a", &get_default_schema()).unwrap();
        save_schema(&engine, "schema-b", &get_default_schema()).unwrap();

        let names = list_schemas(&engine).unwrap();
        assert!(names.contains(&"schema-a".to_string()));
        assert!(names.contains(&"schema-b".to_string()));
    }

    #[test]
    fn test_delete_schema() {
        let engine = create_note_engine();
        save_schema(&engine, "to-delete", &get_default_schema()).unwrap();
        delete_schema(&engine, "to-delete").unwrap();

        let loaded = load_schema(&engine, "to-delete").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_register_default_schema() {
        let engine = create_note_engine();
        register_default_schema(&engine).unwrap();

        let loaded = load_schema(&engine, "default").unwrap();
        assert!(loaded.is_some());

        // Second call should be idempotent
        register_default_schema(&engine).unwrap();
        let names = list_schemas(&engine).unwrap();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn test_field_type_name() {
        assert_eq!(FieldType::String.name(), "string");
        assert_eq!(FieldType::Number.name(), "number");
        assert_eq!(FieldType::Date.name(), "date");
        assert_eq!(FieldType::TagList.name(), "tag list");
        assert_eq!(FieldType::StringList.name(), "string list");
    }
}
