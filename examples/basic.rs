/// Basic usage example: open → put → get → delete.
///
/// Run with:
/// ```bash
/// cargo run --example basic
/// ```
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use apexstore::core::engine::Engine;
use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;

fn main() {
    // Create a unique temporary directory for this run
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let tmp = PathBuf::from(format!("/tmp/apexstore_example_{}", ts));
    std::fs::create_dir_all(&tmp).expect("failed to create temp dir");

    // Build configuration pointing to our temp directory
    let config = LsmConfig::builder()
        .dir_path(tmp.clone())
        .memtable_max_size(4 * 1024 * 1024) // 4 MiB
        .block_size(4096)
        .block_cache_size_mb(16)
        .build()
        .expect("invalid config");

    // Open the engine
    let cache = GlobalBlockCache::new(100, 4096);
    let mut engine = Engine::new_from_config(&config, cache).expect("failed to open engine");

    // ── Put ──
    engine
        .set(b"greeting", b"hello, world!")
        .expect("put failed");
    println!("✓ Put  : greeting → hello, world!");

    // ── Get ──
    let got = engine
        .get(b"greeting")
        .expect("get failed")
        .expect("key not found");
    assert_eq!(got, b"hello, world!");
    println!("✓ Get  : greeting → {}", String::from_utf8_lossy(&got));

    // ── Delete ──
    engine.delete(b"greeting").expect("delete failed");
    let after_delete = engine.get(b"greeting").expect("get after delete failed");
    assert!(after_delete.is_none());
    println!("✓ Del  : greeting gone after delete");

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp);
    println!("✓ All operations completed successfully");
}
