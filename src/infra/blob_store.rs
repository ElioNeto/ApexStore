//! Built-in blob/attachment storage — chunked large-file storage on top of the KV store.
//!
//! This module provides:
//!
//! - [`BlobStore`] — stores large binary data as chunks in the KV engine.
//! - [`BlobStoreConfig`] — configuration including max chunk size.

use std::sync::Arc;

/// Default maximum chunk size in bytes (256 KiB).
const DEFAULT_MAX_CHUNK_SIZE: usize = 256 * 1024;
/// Internal prefix used for blob metadata.
const BLOB_META_PREFIX: &str = "__blob_meta:";
/// Internal prefix used for blob chunks.
const BLOB_CHUNK_PREFIX: &str = "__blob_chunk:";

/// Configuration for a [`BlobStore`].
#[derive(Debug, Clone)]
pub struct BlobStoreConfig {
    /// Maximum size of each chunk in bytes (default: 256 KiB).
    pub max_chunk_size: usize,
}

impl Default for BlobStoreConfig {
    fn default() -> Self {
        Self {
            max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
        }
    }
}

/// A blob storage layer that splits large binary payloads into chunks
/// and stores them in the underlying KV engine.
///
/// Each blob is stored as:
/// - A metadata key `__blob_meta:<name>` → JSON with chunk count and total size.
/// - One or more chunk keys `__blob_chunk:<name>:<seq>` → raw chunk bytes.
pub struct BlobStore {
    /// Reference to the underlying engine (boxed trait so any engine can be used).
    engine: Arc<dyn BlobEngine + Send + Sync>,
    config: BlobStoreConfig,
}

/// Trait abstracting the KV operations needed by [`BlobStore`].
pub trait BlobEngine {
    /// Set a key to a value.
    fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    /// Get a value by key.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>>;
    /// Delete a key.
    fn delete(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Metadata stored for each blob.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct BlobMeta {
    /// Total size of the original blob in bytes.
    total_size: u64,
    /// Number of chunks stored.
    chunk_count: u32,
}

impl BlobStore {
    /// Create a new `BlobStore` wrapping the given engine with default config.
    pub fn new(engine: Arc<dyn BlobEngine + Send + Sync>) -> Self {
        Self {
            engine,
            config: BlobStoreConfig::default(),
        }
    }

    /// Create a new `BlobStore` with a custom configuration.
    pub fn with_config(
        engine: Arc<dyn BlobEngine + Send + Sync>,
        config: BlobStoreConfig,
    ) -> Self {
        Self { engine, config }
    }

    /// Store a blob under the given name.
    ///
    /// The data is split into chunks of at most `max_chunk_size` bytes.
    /// Returns the number of chunks written.
    pub fn store(&self, name: &str, data: &[u8]) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
        let chunk_size = self.config.max_chunk_size;
        let total_size = data.len() as u64;
        let chunk_count = if data.is_empty() {
            1
        } else {
            ((data.len() + chunk_size - 1) / chunk_size) as u32
        };

        // Write each chunk.
        for i in 0..chunk_count {
            let start = (i as usize) * chunk_size;
            let end = std::cmp::min(start + chunk_size, data.len());
            let chunk_key = format!("{}{}:{}", BLOB_CHUNK_PREFIX, name, i);
            self.engine.set(chunk_key.as_bytes(), &data[start..end])?;
        }

        // Write metadata.
        let meta = BlobMeta {
            total_size,
            chunk_count,
        };
        let meta_json = serde_json::to_vec(&meta)?;
        let meta_key = format!("{}{}", BLOB_META_PREFIX, name);
        self.engine.set(meta_key.as_bytes(), &meta_json)?;

        Ok(chunk_count)
    }

    /// Retrieve a blob by name.
    ///
    /// Returns `None` if the blob does not exist.
    pub fn retrieve(&self, name: &str) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
        let meta_key = format!("{}{}", BLOB_META_PREFIX, name);
        let meta_bytes = match self.engine.get(meta_key.as_bytes())? {
            Some(b) => b,
            None => return Ok(None),
        };

        let meta: BlobMeta = serde_json::from_slice(&meta_bytes)?;
        let mut result = Vec::with_capacity(meta.total_size as usize);

        for i in 0..meta.chunk_count {
            let chunk_key = format!("{}{}:{}", BLOB_CHUNK_PREFIX, name, i);
            let chunk = self
                .engine
                .get(chunk_key.as_bytes())?
                .unwrap_or_default();
            result.extend_from_slice(&chunk);
        }

        Ok(Some(result))
    }

    /// Delete a blob and all its chunks.
    pub fn delete(&self, name: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let meta_key = format!("{}{}", BLOB_META_PREFIX, name);

        // Try to read metadata to know chunk count.
        if let Some(meta_bytes) = self.engine.get(meta_key.as_bytes())? {
            if let Ok(meta) = serde_json::from_slice::<BlobMeta>(&meta_bytes) {
                for i in 0..meta.chunk_count {
                    let chunk_key = format!("{}{}:{}", BLOB_CHUNK_PREFIX, name, i);
                    self.engine.delete(chunk_key.as_bytes())?;
                }
            }
        }

        self.engine.delete(meta_key.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// An in-memory engine for testing.
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

    impl BlobEngine for MemEngine {
        fn set(&self, key: &[u8], value: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut map = self.data.lock().unwrap();
            map.insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
            let map = self.data.lock().unwrap();
            Ok(map.get(key).cloned())
        }

        fn delete(&self, key: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut map = self.data.lock().unwrap();
            map.remove(key);
            Ok(())
        }
    }

    #[test]
    fn test_store_and_retrieve_small() {
        let engine = Arc::new(MemEngine::new());
        let store = BlobStore::new(engine);
        store.store("hello.txt", b"Hello, world!").unwrap();
        let result = store.retrieve("hello.txt").unwrap().unwrap();
        assert_eq!(result, b"Hello, world!");
    }

    #[test]
    fn test_store_and_retrieve_large() {
        let engine = Arc::new(MemEngine::new());
        let config = BlobStoreConfig {
            max_chunk_size: 16, // tiny chunks for testing
        };
        let store = BlobStore::with_config(engine, config);
        let data: Vec<u8> = (0..100).map(|i| (i % 256) as u8).collect();
        let chunks = store.store("large.bin", &data).unwrap();
        assert!(chunks > 1); // should be split into multiple chunks
        let result = store.retrieve("large.bin").unwrap().unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_retrieve_missing() {
        let engine = Arc::new(MemEngine::new());
        let store = BlobStore::new(engine);
        assert!(store.retrieve("nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_delete() {
        let engine = Arc::new(MemEngine::new());
        let store = BlobStore::new(engine);
        store.store("temp.txt", b"temporary").unwrap();
        assert!(store.retrieve("temp.txt").unwrap().is_some());
        store.delete("temp.txt").unwrap();
        assert!(store.retrieve("temp.txt").unwrap().is_none());
    }

    #[test]
    fn test_empty_blob() {
        let engine = Arc::new(MemEngine::new());
        let store = BlobStore::new(engine);
        store.store("empty.bin", b"").unwrap();
        let result = store.retrieve("empty.bin").unwrap().unwrap();
        assert!(result.is_empty());
    }
}
