//! CLI SCAN command pagination tests

use apexstore::LsmEngine;
use tempfile::tempdir;

/// Helper to create an isolated engine instance
fn create_test_engine(base_path: &std::path::Path) -> Result<apexstore::LsmConfig, apexstore::infra::error::LsmError> {
    apexstore::LsmConfig::builder()
        .dir_path(base_path.to_path_buf())
        .memtable_max_size(4 * 1024) // 4KB
        .build()
}

#[test]
fn test_cli_scan_pagination_basic() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let engine = LsmEngine::new(config)?;

    for i in 1..=15 {
        engine.set(format!("a:{}", i), format!("v{}", i).as_bytes().to_vec())?;
    }

    let limit = 5;
    let mut all_keys: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let (results, next_cursor) = engine.scan_range(cursor.as_deref(), None, limit)?;
        let keys: Vec<String> = results.into_iter().map(|(k, _)| k).collect();
        all_keys.extend(keys);
        if next_cursor.is_none() {
            break;
        }
        cursor = next_cursor;
    }

    all_keys.sort();
    all_keys.dedup();

    assert_eq!(all_keys.len(), 15, "Should retrieve all 15 keys");
    Ok(())
}

#[test]
fn test_cli_scan_pagination_cursor() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let engine = LsmEngine::new(config)?;

    for i in 1..=10 {
        engine.set(format!("k:{}", i), format!("{}", i).as_bytes().to_vec())?;
    }

    let limit = 3;
    let (_page1, cursor1) = engine.scan_range(None, None, limit)?;
    assert_eq!(cursor1.as_ref().unwrap(), "k:3");

    let (page2, cursor2) = engine.scan_range(cursor1.as_deref(), None, limit)?;
    assert_eq!(page2[0].0, "k:4");

    let (page3, _) = engine.scan_range(cursor2.as_deref(), None, limit)?;
    assert_eq!(page3.len(), 4);

    Ok(())
}

#[test]
fn test_cli_prefix_search_pagination() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let engine = LsmEngine::new(config)?;

    let users = vec!["user:alice", "user:bob", "user:charlie", "user:david"];
    for key in &users {
        engine.set(key.to_string(), b"user_value".to_vec())?;
    }

    let limit = 2;
    let (_page1, cursor1) = engine.search_prefix("user:", None, limit)?;
    let (page2, _) = engine.search_prefix("user:", cursor1.as_deref(), limit)?;

    assert_eq!(page2.len(), 2, "Page 2 should have remaining records");
    Ok(())
}

#[test]
fn test_scan_range_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let engine = LsmEngine::new(config)?;

    for i in 1..=20 {
        engine.set(format!("a:{}", i), format!("{}", i).as_bytes().to_vec())?;
    }

    let (page, _cursor) = engine.scan_range(Some("a:0"), Some("a:5"), 100)?;
    let keys: Vec<&str> = page.iter().map(|(k, _)| k.as_str()).collect();

    assert!(keys.iter().all(|k| k >= "a:0" && k < "a:5"));
    assert_eq!(keys.len(), 4); // a:1, a:2, a:3, a:4

    Ok(())
}
