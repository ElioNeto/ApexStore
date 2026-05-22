//! GraphQL API for ApexStore — flexible query interface.
//!
//! Provides a GraphQL endpoint at `/graphql` and a playground at
//! `/graphql/playground` alongside the existing REST API.

use crate::core::engine::LsmEngine;
use async_graphql::*;
use std::sync::Arc;

/// GraphQL schema type for the ApexStore engine.
pub type AppSchema = Schema<Query, Mutation, EmptySubscription>;

/// Build the GraphQL schema with the given engine.
pub fn build_schema(engine: Arc<LsmEngine>) -> AppSchema {
    Schema::build(Query, Mutation, EmptySubscription)
        .data(engine)
        .finish()
}

/// A key-value pair returned by scan operations.
#[derive(SimpleObject)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

/// JSON-serializable LSM engine statistics.
#[derive(SimpleObject)]
pub struct LsmStatsJson {
    pub sst_files: usize,
    pub sst_kb: usize,
    pub mem_records: usize,
    pub mem_kb: usize,
    pub wal_kb: usize,
    pub total_records: usize,
    pub max_levels_reached: usize,
}

/// GraphQL root query.
pub struct Query;

#[Object]
impl Query {
    /// Get the value for a given key.
    async fn get(&self, ctx: &Context<'_>, key: String) -> Option<String> {
        let engine = ctx.data::<Arc<LsmEngine>>().ok()?;
        match engine.get(key.as_bytes()) {
            Ok(Some(value)) => Some(String::from_utf8_lossy(&value).to_string()),
            _ => None,
        }
    }

    /// Scan all keys, up to an optional limit.
    async fn scan(&self, ctx: &Context<'_>, limit: Option<i32>) -> Vec<KeyValue> {
        let engine = ctx.data::<Arc<LsmEngine>>().ok();
        let engine = match engine {
            Some(e) => e,
            None => return Vec::new(),
        };

        let limit = limit
            .map(|l| l.max(1) as usize)
            .unwrap_or(crate::core::engine::DEFAULT_SCAN_LIMIT);

        match engine.scan_cf("default", None, None, Some(limit)) {
            Ok(results) => results
                .into_iter()
                .map(|(k, v)| KeyValue {
                    key: String::from_utf8_lossy(&k).to_string(),
                    value: String::from_utf8_lossy(&v).to_string(),
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// List all keys.
    async fn keys(&self, ctx: &Context<'_>) -> Vec<String> {
        let engine = ctx.data::<Arc<LsmEngine>>().ok();
        let engine = match engine {
            Some(e) => e,
            None => return Vec::new(),
        };

        match engine.keys() {
            Ok(keys) => keys
                .into_iter()
                .map(|k| String::from_utf8_lossy(&k).to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Get LSM engine statistics.
    async fn stats(&self, ctx: &Context<'_>) -> Option<LsmStatsJson> {
        let engine = ctx.data::<Arc<LsmEngine>>().ok()?;
        match engine.stats("default") {
            Ok(stats) => Some(LsmStatsJson {
                sst_files: stats.sst_files,
                sst_kb: stats.sst_kb,
                mem_records: stats.mem_records,
                mem_kb: stats.mem_kb,
                wal_kb: stats.wal_kb,
                total_records: stats.total_records,
                max_levels_reached: stats.max_levels_reached,
            }),
            Err(_) => None,
        }
    }
}

/// GraphQL root mutation.
pub struct Mutation;

#[Object]
impl Mutation {
    /// Set a key-value pair.
    async fn set(&self, ctx: &Context<'_>, key: String, value: String) -> bool {
        let engine = ctx.data::<Arc<LsmEngine>>().ok();
        let engine = match engine {
            Some(e) => e,
            None => return false,
        };

        engine
            .set(key.as_bytes().to_vec(), value.as_bytes().to_vec())
            .is_ok()
    }

    /// Delete a key.
    async fn delete(&self, ctx: &Context<'_>, key: String) -> bool {
        let engine = ctx.data::<Arc<LsmEngine>>().ok();
        let engine = match engine {
            Some(e) => e,
            None => return false,
        };

        engine.delete(key.as_bytes()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::storage::cache::GlobalBlockCache;

    #[test]
    fn test_graphql_schema_builds() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Arc::new(
            crate::core::engine::Engine::new_from_config(
                &config,
                GlobalBlockCache::new(100, 4096),
            )
            .unwrap(),
        );
        let schema = build_schema(engine);
        let sdl = schema.sdl();
        assert!(sdl.contains("get"));
        assert!(sdl.contains("scan"));
        assert!(sdl.contains("keys"));
        assert!(sdl.contains("stats"));
        assert!(sdl.contains("set"));
        assert!(sdl.contains("delete"));
    }

    #[test]
    fn test_graphql_query_get_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Arc::new(
            crate::core::engine::Engine::new_from_config(
                &config,
                GlobalBlockCache::new(100, 4096),
            )
            .unwrap(),
        );
        let schema = build_schema(engine.clone());

        let res = futures::executor::block_on(
            schema.execute("{ get(key: \"nonexistent\") }"),
        );
        assert!(res.errors.is_empty());
    }

    #[test]
    fn test_graphql_mutation_set_and_get() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Arc::new(
            crate::core::engine::Engine::new_from_config(
                &config,
                GlobalBlockCache::new(100, 4096),
            )
            .unwrap(),
        );
        let schema = build_schema(engine.clone());

        // Insert via mutation
        let res = futures::executor::block_on(
            schema.execute(r#"mutation { set(key: "hello", value: "world") }"#),
        );
        assert!(res.errors.is_empty());
        let data = res.data.into_json().unwrap();
        assert_eq!(data["set"], true);

        // Query via get
        let res = futures::executor::block_on(
            schema.execute(r#"{ get(key: "hello") }"#),
        );
        assert!(res.errors.is_empty());
        let data = res.data.into_json().unwrap();
        assert_eq!(data["get"], "world");
    }

    #[test]
    fn test_graphql_mutation_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Arc::new(
            crate::core::engine::Engine::new_from_config(
                &config,
                GlobalBlockCache::new(100, 4096),
            )
            .unwrap(),
        );
        let schema = build_schema(engine.clone());

        // Insert
        let _ = futures::executor::block_on(
            schema.execute(r#"mutation { set(key: "todelete", value: "x") }"#),
        );

        // Delete
        let res = futures::executor::block_on(
            schema.execute(r#"mutation { delete(key: "todelete") }"#),
        );
        assert!(res.errors.is_empty());
        let data = res.data.into_json().unwrap();
        assert_eq!(data["delete"], true);

        // Verify gone
        let res = futures::executor::block_on(
            schema.execute(r#"{ get(key: "todelete") }"#),
        );
        let data = res.data.into_json().unwrap();
        assert_eq!(data["get"], serde_json::Value::Null);
    }
}
