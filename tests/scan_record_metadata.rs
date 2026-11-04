use apexstore::storage::cache::GlobalBlockCache;
use apexstore::{LsmConfig, LsmEngine};
use proptest::prelude::*;
use std::collections::BTreeMap;

fn open(config: &LsmConfig) -> LsmEngine {
    LsmEngine::new_from_config(config, GlobalBlockCache::new(100, 4096)).unwrap()
}

#[test]
fn scan_reads_flushed_records_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    {
        let engine = open(&config);
        engine.set("mango", b"persisted").unwrap();
        engine.flush_memtable().unwrap();
    }
    let engine = open(&config);
    assert_eq!(engine.get("mango").unwrap(), Some(b"persisted".to_vec()));
    assert_eq!(
        engine.scan().unwrap(),
        vec![(b"mango".to_vec(), b"persisted".to_vec())]
    );
    assert_eq!(
        engine
            .scan_range("default", b"mango", b"zebra", Some(1))
            .unwrap(),
        vec![(b"mango".to_vec(), b"persisted".to_vec())]
    );
}

#[test]
fn scan_preserves_writes_newer_than_a_range_delete() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let engine = open(&config);
    engine.set("mango", b"before").unwrap();
    engine.delete_range(b"apple", b"zebra").unwrap();
    engine.set("mango", b"after").unwrap();
    assert_eq!(engine.get("mango").unwrap(), Some(b"after".to_vec()));
    assert_eq!(
        engine.scan().unwrap(),
        vec![(b"mango".to_vec(), b"after".to_vec())]
    );
    engine.flush_memtable().unwrap();
    assert_eq!(engine.get("mango").unwrap(), Some(b"after".to_vec()));
    assert_eq!(
        engine.scan().unwrap(),
        vec![(b"mango".to_vec(), b"after".to_vec())]
    );
}

#[test]
fn index_catalog_survives_flush_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    {
        let engine = open(&config);
        engine.create_index("default", "kind").unwrap();
        assert_eq!(engine.list_indexes("default").unwrap(), vec!["kind"]);
        engine.flush_memtable().unwrap();
    }
    let engine = open(&config);
    assert_eq!(
        engine.get("__idx:__schema:index:kind").unwrap(),
        Some(b"1".to_vec())
    );
    assert_eq!(engine.list_indexes("default").unwrap(), vec!["kind"]);
}

#[test]
fn deleted_and_expired_versions_do_not_consume_scan_limits() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let engine = open(&config);
    engine.set("a", b"old").unwrap();
    engine.flush_memtable().unwrap();
    engine.delete("a").unwrap();
    engine
        .set_with_ttl("b", b"expired", std::time::Duration::ZERO)
        .unwrap();
    engine.set("c", b"live").unwrap();
    assert_eq!(engine.get("a").unwrap(), None);
    assert_eq!(engine.get("b").unwrap(), None);
    assert_eq!(
        engine.scan_cf("default", None, None, Some(1)).unwrap(),
        vec![(b"c".to_vec(), b"live".to_vec())]
    );
    assert_eq!(engine.keys().unwrap(), vec![b"c".to_vec()]);
    assert_eq!(engine.count().unwrap(), 1);
}

#[test]
fn range_only_flush_keeps_the_deletion_durable() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    {
        let engine = open(&config);
        engine.set("mango", b"old").unwrap();
        engine.flush_memtable().unwrap();
        engine.delete_range(b"apple", b"zebra").unwrap();
        engine.flush_memtable().unwrap();
        engine.flush_memtable().unwrap();
    }
    let engine = open(&config);
    assert_eq!(engine.get("mango").unwrap(), None);
    assert!(engine.scan().unwrap().is_empty());
    assert!(engine.keys().unwrap().is_empty());
    assert_eq!(engine.count().unwrap(), 0);
}

#[test]
fn missing_sstable_is_a_scan_error_instead_of_missing_data() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let engine = open(&config);
    engine.set("mango", b"persisted").unwrap();
    engine.flush_memtable().unwrap();
    let files: Vec<_> = std::fs::read_dir(directory.path().join("sstables"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sst"))
        .collect();
    assert_eq!(files.len(), 1);
    std::fs::remove_file(&files[0]).unwrap();
    assert!(engine.scan().is_err());
    assert!(engine.keys().is_err());
    assert!(engine.count().is_err());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn point_and_range_reads_match_the_model(
        operations in prop::collection::vec((0u8..3, 0u8..4), 1..25)
    ) {
        let directory = tempfile::tempdir().unwrap();
        let config = LsmConfig::builder().dir_path(directory.path().to_path_buf()).build().unwrap();
        let engine = open(&config);
        let mut expected = BTreeMap::new();
        for (sequence, (operation, index)) in operations.into_iter().enumerate() {
            let key = vec![b'k', b'a' + index];
            match operation {
                0 => {
                    let value = format!("value-{sequence}").into_bytes();
                    engine.set(key.clone(), value.clone()).unwrap();
                    expected.insert(key, value);
                }
                1 => {
                    engine.delete_range(b"ka", b"kd").unwrap();
                    expected.retain(|key, _| key.as_slice() < b"ka".as_slice() || key.as_slice() >= b"kd".as_slice());
                }
                _ => engine.flush_memtable().unwrap(),
            }
            for suffix in b'a'..=b'd' {
                let key = vec![b'k', suffix];
                prop_assert_eq!(engine.get(&key).unwrap(), expected.get(&key).cloned());
            }
            let scan: BTreeMap<_, _> = engine.scan().unwrap().into_iter().collect();
            prop_assert_eq!(&scan, &expected);
            prop_assert_eq!(engine.keys().unwrap(), expected.keys().cloned().collect::<Vec<_>>());
            prop_assert_eq!(engine.count().unwrap(), expected.len());
        }
    }
}

#[test]
fn transaction_reads_respect_range_delete_ordering() {
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let engine = open(&config);
    engine.set("mango", b"before").unwrap();
    engine.delete_range(b"apple", b"zebra").unwrap();
    assert_eq!(engine.begin_transaction().get("mango").unwrap(), None);
    engine.set("mango", b"after").unwrap();
    assert_eq!(
        engine.begin_transaction().get("mango").unwrap(),
        Some(b"after".to_vec())
    );
    engine.flush_memtable().unwrap();
    assert_eq!(
        engine.begin_transaction().get("mango").unwrap(),
        Some(b"after".to_vec())
    );
    let mut transaction = engine.begin_transaction();
    transaction.put("mango", b"buffered").unwrap();
    assert_eq!(
        transaction.get("mango").unwrap(),
        Some(b"buffered".to_vec())
    );
    transaction.rollback();
}

#[test]
fn sql_point_and_scan_queries_preserve_newer_flushed_writes() {
    use apexstore::infra::sql::{SqlEngine, SqlResult};
    let directory = tempfile::tempdir().unwrap();
    let config = LsmConfig::builder()
        .dir_path(directory.path().to_path_buf())
        .build()
        .unwrap();
    let engine = open(&config);
    engine.set("mango", b"before").unwrap();
    engine.delete_range(b"apple", b"zebra").unwrap();
    engine.set("mango", b"after").unwrap();
    engine.flush_memtable().unwrap();
    let sql = SqlEngine::new(&engine);
    for query in [
        "SELECT * FROM default",
        "SELECT * FROM default WHERE key = 'mango'",
    ] {
        let SqlResult::Rows { data, .. } = sql.execute(query).unwrap() else {
            panic!("SELECT must return rows");
        };
        assert_eq!(data, vec![vec!["mango".to_string(), "after".to_string()]]);
    }
}
