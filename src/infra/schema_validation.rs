//! Schema-on-write validation — JSON Schema validation for key-value writes.
//!
//! This module provides:
//!
//! - [`SchemaValidator`] — registers JSON schemas for key prefixes and
//!   validates values on write.
//! - [`ValidationError`] — error type for validation failures.

use std::collections::HashMap;

/// Error returned when a value does not conform to its registered schema.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// The key that failed validation.
    pub key: Vec<u8>,
    /// A human-readable description of the failure.
    pub reason: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "schema validation failed for key {:?}: {}",
            String::from_utf8_lossy(&self.key),
            self.reason
        )
    }
}

impl std::error::Error for ValidationError {}

/// A type alias for validation results.
pub type ValidationResult = Result<(), ValidationError>;

/// Validates values against registered JSON schemas on write.
///
/// Schemas are registered with a key prefix. When a value is written with a
/// key matching that prefix, the value is validated against the schema.
pub struct SchemaValidator {
    /// Map from key prefix to compiled JSON Schema.
    schemas: HashMap<String, serde_json::Value>,
}

impl SchemaValidator {
    /// Create a new empty schema validator.
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
        }
    }

    /// Register a JSON schema for a key prefix.
    ///
    /// The `schema_json` must be a valid JSON Schema object (draft-07).
    /// Returns an error if the schema is not valid JSON or is not an object.
    pub fn register_schema(
        &mut self,
        key_prefix: &str,
        schema_json: serde_json::Value,
    ) -> Result<(), String> {
        // Basic validation: must be a JSON object (schema).
        if !schema_json.is_object() {
            return Err("schema must be a JSON object".to_string());
        }
        self.schemas.insert(key_prefix.to_string(), schema_json);
        Ok(())
    }

    /// Remove a previously registered schema for a key prefix.
    pub fn remove_schema(&mut self, key_prefix: &str) {
        self.schemas.remove(key_prefix);
    }

    /// Validate a `(key, value)` pair against its matching schema.
    ///
    /// Returns `Ok(())` if the value is valid or no schema matches the key.
    /// Returns `Err(ValidationError)` if validation fails.
    ///
    /// The value is expected to be valid JSON. If it cannot be parsed as JSON,
    /// validation fails with a parse error.
    pub fn validate(&self, key: &[u8], value: &[u8]) -> ValidationResult {
        let key_str = String::from_utf8_lossy(key);

        // Find the longest matching prefix.
        let matching_schema = self
            .schemas
            .iter()
            .filter(|(prefix, _)| key_str.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len());

        let (_prefix, schema) = match matching_schema {
            Some(s) => s,
            None => return Ok(()), // no matching schema
        };

        // Parse the value as JSON.
        let instance: serde_json::Value = match serde_json::from_slice(value) {
            Ok(v) => v,
            Err(e) => {
                return Err(ValidationError {
                    key: key.to_vec(),
                    reason: format!("value is not valid JSON: {}", e),
                });
            }
        };

        // Validate against the schema using jsonschema.
        let compiled: jsonschema::JSONSchema = match jsonschema::JSONSchema::compile(schema) {
            Ok(v) => v,
            Err(e) => {
                return Err(ValidationError {
                    key: key.to_vec(),
                    reason: format!("invalid schema definition: {}", e),
                });
            }
        };

        if let Err(errors) = compiled.validate(&instance) {
            let reasons: Vec<String> = errors.into_iter().map(|e| format!("{}", e)).collect();
            return Err(ValidationError {
                key: key.to_vec(),
                reason: reasons.join("; "),
            });
        }

        Ok(())
    }

    /// Return `true` if a schema is registered for the given prefix.
    pub fn has_schema(&self, key_prefix: &str) -> bool {
        self.schemas.contains_key(key_prefix)
    }

    /// Return the number of registered schemas.
    pub fn schema_count(&self) -> usize {
        self.schemas.len()
    }
}

impl Default for SchemaValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer", "minimum": 0 }
            },
            "required": ["name"]
        })
    }

    #[test]
    fn test_register_and_validate_valid() {
        let mut validator = SchemaValidator::new();
        validator.register_schema("users/", schema()).unwrap();

        let value = serde_json::json!({"name": "Alice", "age": 30});
        let result = validator.validate(b"users/123", value.to_string().as_bytes());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_invalid() {
        let mut validator = SchemaValidator::new();
        validator.register_schema("users/", schema()).unwrap();

        // Missing required "name"
        let value = serde_json::json!({"age": 30});
        let result = validator.validate(b"users/123", value.to_string().as_bytes());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.reason.contains("name"));
    }

    #[test]
    fn test_no_matching_schema() {
        let mut validator = SchemaValidator::new();
        validator.register_schema("users/", schema()).unwrap();

        let value = serde_json::json!({"anything": "goes"});
        let result = validator.validate(b"other/key", value.to_string().as_bytes());
        assert!(result.is_ok()); // no schema for "other/" prefix
    }

    #[test]
    fn test_non_json_value() {
        let mut validator = SchemaValidator::new();
        validator
            .register_schema("raw/", serde_json::json!({"type": "string"}))
            .unwrap();

        let result = validator.validate(b"raw/data", b"not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_schema() {
        let mut validator = SchemaValidator::new();
        validator
            .register_schema("test/", serde_json::json!({"type": "object"}))
            .unwrap();
        assert!(validator.has_schema("test/"));
        validator.remove_schema("test/");
        assert!(!validator.has_schema("test/"));
    }

    #[test]
    fn test_schema_count() {
        let mut validator = SchemaValidator::new();
        assert_eq!(validator.schema_count(), 0);
        validator
            .register_schema("a/", serde_json::json!({"type": "object"}))
            .unwrap();
        validator
            .register_schema("b/", serde_json::json!({"type": "string"}))
            .unwrap();
        assert_eq!(validator.schema_count(), 2);
    }

    #[test]
    fn test_longest_prefix_wins() {
        let mut validator = SchemaValidator::new();
        validator
            .register_schema("users/", serde_json::json!({"type": "object"}))
            .unwrap();
        validator
            .register_schema(
                "users/admin/",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "role": { "const": "admin" }
                    },
                    "required": ["role"]
                }),
            )
            .unwrap();

        // Should match the longer prefix
        let value = serde_json::json!({"name": "Bob", "role": "admin"});
        let result = validator.validate(b"users/admin/1", value.to_string().as_bytes());
        assert!(result.is_ok());

        // Missing "role" should fail against the admin schema
        let bad_value = serde_json::json!({"name": "Bob"});
        let result = validator.validate(b"users/admin/1", bad_value.to_string().as_bytes());
        assert!(result.is_err());
    }
}
