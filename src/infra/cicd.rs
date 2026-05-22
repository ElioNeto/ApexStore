//! Built-in CI/CD integration — test fixtures and seed data management.
//!
//! This module provides:
//!
//! - [`TestFixture`] — manages named test fixtures for CI/CD pipelines.
//! - [`FixtureEntry`] — a single key-value entry within a fixture.

use std::collections::HashMap;

/// A single key-value entry within a fixture.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureEntry {
    /// The key.
    pub key: Vec<u8>,
    /// The value.
    pub value: Vec<u8>,
}

/// A named fixture containing a set of key-value pairs.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// The name of this fixture.
    pub name: String,
    /// The key-value entries in this fixture.
    pub entries: Vec<FixtureEntry>,
}

/// A trait abstracting the KV operations needed to load and reset fixtures.
pub trait FixtureEngine: Send + Sync {
    /// Set a key to a value.
    fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// Delete a key.
    fn delete(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// List all keys in the store.
    fn keys(&self) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Manages test fixtures for CI/CD pipelines.
///
/// Provides helpers to load predefined fixtures, seed data, and reset the
/// engine state between test runs.
pub struct TestFixture {
    engine: Box<dyn FixtureEngine>,
    fixtures: HashMap<String, Fixture>,
}

impl TestFixture {
    /// Create a new `TestFixture` wrapping the given engine.
    pub fn new(engine: Box<dyn FixtureEngine>) -> Self {
        Self {
            engine,
            fixtures: HashMap::new(),
        }
    }

    /// Register a fixture so it can be loaded later by name.
    pub fn register_fixture(&mut self, fixture: Fixture) {
        self.fixtures.insert(fixture.name.clone(), fixture);
    }

    /// Load a fixture by name, inserting all its entries into the engine.
    ///
    /// Returns `None` if no fixture with that name has been registered.
    pub fn load_fixture(&self, name: &str) -> Result<Option<()>, Box<dyn std::error::Error + Send + Sync>> {
        match self.fixtures.get(name) {
            Some(fixture) => {
                for entry in &fixture.entries {
                    self.engine.set(&entry.key, &entry.value)?;
                }
                Ok(Some(()))
            }
            None => Ok(None),
        }
    }

    /// Seed data into the engine using an explicit list of entries
    /// (inline, no named fixture needed).
    pub fn seed_data(
        &self,
        entries: &[FixtureEntry],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for entry in entries {
            self.engine.set(&entry.key, &entry.value)?;
        }
        Ok(())
    }

    /// Reset the engine state by deleting all keys.
    pub fn reset_state(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let keys = self.engine.keys()?;
        for key in &keys {
            self.engine.delete(key)?;
        }
        Ok(())
    }

    /// Generate test data with a simple schema and count.
    ///
    /// The `schema` parameter is a template string where `{n}` is replaced
    /// with the counter (e.g., `"key_{n}"` / `"value_{n}"`). Returns the
    /// generated entries without inserting them.
    pub fn generate_test_data(&self, schema: &str, count: u64) -> Vec<FixtureEntry> {
        let mut entries = Vec::with_capacity(count as usize);
        for i in 0..count {
            let key = schema.replace("{n}", &i.to_string());
            let value = format!("value_{}", i);
            entries.push(FixtureEntry {
                key: key.into_bytes(),
                value: value.into_bytes(),
            });
        }
        entries
    }

    /// Return the names of all registered fixtures.
    pub fn fixture_names(&self) -> Vec<String> {
        self.fixtures.keys().cloned().collect()
    }

    /// Remove a fixture from the registry.
    pub fn unregister_fixture(&mut self, name: &str) {
        self.fixtures.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MemEngine {
        data: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
    }

    impl MemEngine {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    impl FixtureEngine for MemEngine {
        fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.data.lock().unwrap().insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }

        fn keys(&self) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.data.lock().unwrap().keys().cloned().collect())
        }
    }

    #[test]
    fn test_load_fixture() {
        let engine = Box::new(MemEngine::new());
        let mut tf = TestFixture::new(engine);

        tf.register_fixture(Fixture {
            name: "test_data".into(),
            entries: vec![
                FixtureEntry {
                    key: b"k1".to_vec(),
                    value: b"v1".to_vec(),
                },
                FixtureEntry {
                    key: b"k2".to_vec(),
                    value: b"v2".to_vec(),
                },
            ],
        });

        assert_eq!(tf.fixture_names(), vec!["test_data"]);
        let result = tf.load_fixture("test_data").unwrap();
        assert!(result.is_some());

        // Second load should succeed (upsert)
        let result = tf.load_fixture("test_data").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_load_missing_fixture() {
        let engine = Box::new(MemEngine::new());
        let tf = TestFixture::new(engine);
        let result = tf.load_fixture("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_seed_data() {
        let engine = Box::new(MemEngine::new());
        let tf = TestFixture::new(engine);

        tf.seed_data(&[FixtureEntry {
            key: b"a".to_vec(),
            value: b"b".to_vec(),
        }])
        .unwrap();
    }

    #[test]
    fn test_reset_state() {
        let engine = Box::new(MemEngine::new());
        let tf = TestFixture::new(engine);

        tf.seed_data(&[FixtureEntry {
            key: b"temp".to_vec(),
            value: b"data".to_vec(),
        }])
        .unwrap();
        tf.reset_state().unwrap();
    }

    #[test]
    fn test_generate_test_data() {
        let engine = Box::new(MemEngine::new());
        let tf = TestFixture::new(engine);
        let data = tf.generate_test_data("key_{n}", 3);
        assert_eq!(data.len(), 3);
        assert_eq!(data[0].key, b"key_0");
        assert_eq!(data[1].key, b"key_1");
        assert_eq!(data[2].key, b"key_2");
    }

    #[test]
    fn test_unregister_fixture() {
        let engine = Box::new(MemEngine::new());
        let mut tf = TestFixture::new(engine);

        tf.register_fixture(Fixture {
            name: "temp".into(),
            entries: vec![],
        });
        assert_eq!(tf.fixture_names().len(), 1);
        tf.unregister_fixture("temp");
        assert!(tf.fixture_names().is_empty());
    }
}
