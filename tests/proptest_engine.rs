use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::LsmEngine;
use proptest::prelude::*;
use tempfile::TempDir;

fn key_value_pairs() -> impl Strategy<Value = Vec<(Vec<u8>, Vec<u8>)>> {
    proptest::collection::vec(
        (
            proptest::collection::vec(proptest::arbitrary::any::<u8>(), 1..=16),
            proptest::collection::vec(proptest::arbitrary::any::<u8>(), 1..=64),
        ),
        1..=5,
    )
}

fn bounded_key() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(proptest::arbitrary::any::<u8>(), 1..=16)
}

fn bounded_value() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(proptest::arbitrary::any::<u8>(), 0..=64)
}

proptest! {
    #[test]
    fn test_put_get_roundtrip(key in bounded_key(), value in bounded_value()) {
        // Skip empty keys (engine doesn't support them)
        prop_assume!(!key.is_empty());
        prop_assume!(key.len() <= 1024);
        prop_assume!(value.len() <= 4096);

        let dir = TempDir::new().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();

        engine.put_cf("default", key.clone(), value.clone()).unwrap();
        let result = engine.get_cf("default", &key).unwrap();

        prop_assert_eq!(result, Some(value));
    }

    #[test]
    fn test_put_delete_get(key in bounded_key(), value in bounded_value()) {
        let dir = TempDir::new().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();

        engine.put_cf("default", key.clone(), value).unwrap();
        engine.delete_cf("default", key.as_slice()).unwrap();
        let result = engine.get_cf("default", &key).unwrap();

        prop_assert_eq!(result, None);
    }

    #[test]
    fn test_put_overwrite(key in bounded_key(), v1 in bounded_value(), v2 in bounded_value()) {

        let dir = TempDir::new().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();

        engine.put_cf("default", key.clone(), v1).unwrap();
        engine.put_cf("default", key.clone(), v2.clone()).unwrap();
        let result = engine.get_cf("default", &key).unwrap();

        // Should return the latest value
        prop_assert_eq!(result, Some(v2));
    }

    #[test]
    fn test_multiple_keys_are_independent(pairs in key_value_pairs()) {

        let dir = TempDir::new().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();

        let mut expected = std::collections::BTreeMap::new();
        for (k, v) in &pairs {
            engine.put_cf("default", k.clone(), v.clone()).unwrap();
            expected.insert(k.clone(), v.clone());
            prop_assert_eq!(engine.get_cf("default", k).unwrap(), Some(v.clone()));
        }

        // Repeated keys overwrite earlier values; retain them in the generated
        // workload and compare against a last-write-wins reference model.
        for (k, v) in &expected {
            let result = engine.get_cf("default", k).unwrap();
            prop_assert_eq!(result, Some(v.clone()), "key {:?} should have value {:?}", k, v);
        }
    }
}
