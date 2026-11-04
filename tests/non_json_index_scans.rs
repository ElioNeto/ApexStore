use apexstore::storage::cache::GlobalBlockCache;
use apexstore::{LsmConfig, LsmEngine};

#[test]
fn binary_writes_and_deletes_do_not_scan_index_schemas() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let values = [
        b"plain text".to_vec(),
        vec![0xff, 0x00, 0x80],
        b"{invalid".to_vec(),
    ];
    {
        let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();
        engine.create_index("default", "kind").unwrap();
        let before = engine.metrics().snapshot().scans;
        for (index, value) in values.iter().enumerate() {
            engine.set(format!("key{index}"), value.clone()).unwrap();
        }
        engine.delete("key1").unwrap();
        assert_eq!(
            engine.metrics().snapshot().scans,
            before,
            "non-JSON values cannot have indexed fields and must not scan the keyspace"
        );
        assert_eq!(engine.get("key0").unwrap(), Some(values[0].clone()));
        assert_eq!(engine.get("key1").unwrap(), None);
        assert_eq!(engine.get("key2").unwrap(), Some(values[2].clone()));
        engine.flush_memtable().unwrap();
    }
    let engine = LsmEngine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();
    assert_eq!(engine.get("key0").unwrap(), Some(values[0].clone()));
    assert_eq!(engine.get("key1").unwrap(), None);
    assert_eq!(engine.get("key2").unwrap(), Some(values[2].clone()));
    assert_eq!(
        engine.get("__idx:__schema:index:kind").unwrap(),
        Some(b"1".to_vec())
    );
}
