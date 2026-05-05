//! CLI SCAN command pagination tests

use apexstore::LsmEngine;
use tempfile::tempdir;

/// Helper to create an isolated engine instance
fn create_test_engine(
    base_path: &std::path::Path,
) -> Result<apexstore::LsmConfig, apexstore::infra::error::LsmError> {
    apexstore::LsmConfig::builder()
        .dir_path(base_path.to_path_buf())
        .memtable_max_size(4 * 1024) // 4KB
        .build()
}

#[test]
fn test_cli_scan_pagination_basic() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let mut engine = LsmEngine::new_from_config(&config, apexstore::storage::cache::GlobalBlockCache::new(100, 4096))?;

    for i in 1..=15 {
        engine.set(format!("a:{}", i), format!("v{}", i).as_bytes().to_vec())?;
    }

    // Use scan_range with limit to get paginated results
    let results = engine.scan_range("default", b"a:0", b"a:~", Some(5))?;
    assert!(results.len() <= 5, "Should return at most 5 results");

    // Get all results without pagination
    let all_results = engine.scan_range("default", b"a:0", b"a:~", Some(100))?;
    assert_eq!(all_results.len(), 15, "Should retrieve all 15 keys");

    Ok(())
}

#[test]
fn test_cli_scan_pagination_cursor() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let mut engine = LsmEngine::new_from_config(&config, apexstore::storage::cache::GlobalBlockCache::new(100, 4096))?;

    for i in 1..=10 {
        engine.set(format!("k:{}", i), format!("{}", i).as_bytes().to_vec())?;
    }

    // Get first page
    let page1 = engine.scan_range("default", b"k:", b"k:~", Some(3))?;
    assert_eq!(page1.len(), 3, "First page should have 3 results");

    // Get second page starting after page1's last key
    if let Some((last_key, _)) = page1.last() {
        let page2 = engine.scan_range("default", &last_key, b"k:~", Some(3))?;
        assert_eq!(page2.len(), 3, "Second page should have 3 results");
    }

    // Get all results
    let all = engine.scan_range("default", b"k:", b"k:~", Some(100))?;
    assert_eq!(all.len(), 10, "Should have all 10 keys");

    Ok(())
}

#[test]
fn test_cli_prefix_search_pagination() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let mut engine = LsmEngine::new_from_config(&config, apexstore::storage::cache::GlobalBlockCache::new(100, 4096))?;

    let users = vec!["user:alice", "user:bob", "user:charlie", "user:david"];
    for key in &users {
        engine.set(key.to_string(), b"user_value".to_vec())?;
    }

    // Use search_prefix with limit and capture the cursor
    let (page1, cursor) = engine.search_prefix("user:", None, 2)?;
    assert_eq!(page1.len(), 2, "First page should have 2 results");

    // Get remaining results using the cursor from the first page
    assert!(cursor.is_some(), "Cursor should be present when there are more records");
    if let Some(ref c) = cursor {
        let (page2, _) = engine.search_prefix("user:", Some(c), 2)?;
        assert_eq!(page2.len(), 2, "Second page should have remaining records");
    }

    Ok(())
}

#[test]
fn test_scan_range_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = tempdir()?;
    let config = create_test_engine(base_dir.path())?;
    let mut engine = LsmEngine::new_from_config(&config, apexstore::storage::cache::GlobalBlockCache::new(100, 4096))?;

    for i in 1..=20 {
        engine.set(format!("a:{}", i), format!("{}", i).as_bytes().to_vec())?;
    }

    let page = engine.scan_range("default", b"a:0", b"a:5", Some(100))?;
    let keys: Vec<String> = page
        .iter()
        .map(|(k, _)| String::from_utf8(k.clone()).unwrap())
        .collect();

    // Lexicographic order: keys starting with "a:1" through "a:19" and "a:2" through "a:4"
    // are all < "a:5" (since "1xxx" < "a:5" and "2xxx" < "5")
    // Keys: a:1, a:10-a:19 (10 keys), a:2-a:4 (3 keys) = 15 total
    assert!(keys
        .iter()
        .all(|k| k.as_str() >= "a:0" && k.as_str() < "a:5"));
    assert_eq!(keys.len(), 15);

    Ok(())
}
