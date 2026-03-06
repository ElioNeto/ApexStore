use apexstore::{LsmConfig, LsmEngine};
use std::env;
use std::io;
use std::path::PathBuf;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load .env file if it exists
    #[cfg(feature = "api")]
    {
        let _ = dotenvy::dotenv();
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    println!("╔═══════════════════════════════════════════════════════╗");
    println!("║         LSM-Tree REST API Server                      ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    // Load server configuration from environment
    let server_config = apexstore::api::ServerConfig::from_env();

    // Load LSM engine configuration from environment
    let data_dir = env::var("DATA_DIR").unwrap_or_else(|_| "./.lsm_data".to_string());

    let memtable_max_size = env::var("MEMTABLE_MAX_SIZE")
        .unwrap_or_else(|_| (4 * 1024 * 1024).to_string())
        .parse::<usize>()
        .unwrap_or(4 * 1024 * 1024);

    let block_size = env::var("BLOCK_SIZE")
        .unwrap_or_else(|_| "4096".to_string())
        .parse::<usize>()
        .unwrap_or(4096);

    let block_cache_size_mb = env::var("BLOCK_CACHE_SIZE_MB")
        .unwrap_or_else(|_| "64".to_string())
        .parse::<usize>()
        .unwrap_or(64);

    let sparse_index_interval = env::var("SPARSE_INDEX_INTERVAL")
        .unwrap_or_else(|_| "16".to_string())
        .parse::<usize>()
        .unwrap_or(16);

    let bloom_false_positive_rate = env::var("BLOOM_FALSE_POSITIVE_RATE")
        .unwrap_or_else(|_| "0.01".to_string())
        .parse::<f64>()
        .unwrap_or(0.01);

    let config = LsmConfig::builder()
        .dir_path(PathBuf::from(&data_dir))
        .memtable_max_size(memtable_max_size)
        .block_size(block_size)
        .block_cache_size_mb(block_cache_size_mb)
        .sparse_index_interval(sparse_index_interval)
        .bloom_false_positive_rate(bloom_false_positive_rate)
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;

    // Print LSM configuration
    println!("📋 LSM Engine Configuration:");
    match PathBuf::from(&data_dir).canonicalize() {
        Ok(abs_path) => println!("   Data Directory: {}", abs_path.display()),
        Err(_) => println!("   Data Directory: {} (will be created)", data_dir),
    }
    println!(
        "   MemTable Max Size: {} MB",
        memtable_max_size / 1024 / 1024
    );
    println!("   Block Size: {} bytes", block_size);
    println!("   Block Cache: {} MB", block_cache_size_mb);
    println!("   Sparse Index Interval: {}", sparse_index_interval);
    println!("   Bloom Filter FP Rate: {}", bloom_false_positive_rate);
    println!();

    let engine = match LsmEngine::new(config) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("❌ Error initializing LSM Engine: {}", e);
            eprintln!(
                "💡 Tip: if you don't need to recover unflushed writes, rename/delete wal.log and try again."
            );
            return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
        }
    };

    println!("✓ Engine initialized successfully!\n");

    apexstore::api::start_server(engine, server_config).await
}
