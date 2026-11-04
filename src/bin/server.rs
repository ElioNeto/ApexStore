use apexstore::infra::telemetry;
use apexstore::{LsmConfig, LsmEngine};
use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load .env file if it exists
    #[cfg(feature = "api")]
    {
        let _ = dotenvy::dotenv();
    }

    // Initialise OpenTelemetry tracing + metrics (falls back to console fmt
    // when OTEL_EXPORTER_OTLP_ENDPOINT is not set).
    let traces = telemetry::init_tracing().map_err(io::Error::other)?;
    let metrics = match telemetry::init_metrics() {
        Ok(metrics) => metrics,
        Err(error) => {
            if let Err(shutdown_error) = telemetry::shutdown(traces, None).await {
                eprintln!("{shutdown_error}");
            }
            return Err(io::Error::other(error));
        }
    };

    let result = run_server().await;
    let shutdown_result = telemetry::shutdown(traces, metrics).await;
    if let Err(error) = &shutdown_result {
        eprintln!("{error}");
    }
    result.and(shutdown_result.map_err(io::Error::other))
}

async fn run_server() -> std::io::Result<()> {
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

    let prefix_compression = env::var("PREFIX_COMPRESSION_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap_or(false);

    let encryption_enabled = match env::var("ENCRYPTION_ENABLED") {
        Ok(value) => value.parse::<bool>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ENCRYPTION_ENABLED must be true or false",
            )
        })?,
        Err(env::VarError::NotPresent) => false,
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    let encryption_key_path = match env::var("ENCRYPTION_KEY_PATH") {
        Ok(value) => Some(value).filter(|path| !path.trim().is_empty()),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };

    let mut builder = LsmConfig::builder()
        .dir_path(PathBuf::from(&data_dir))
        .memtable_max_size(memtable_max_size)
        .block_size(block_size)
        .block_cache_size_mb(block_cache_size_mb)
        .sparse_index_interval(sparse_index_interval)
        .bloom_false_positive_rate(bloom_false_positive_rate)
        .prefix_compression(prefix_compression)
        .encryption_enabled(encryption_enabled);
    if let Some(path) = encryption_key_path {
        builder = builder.encryption_key_path(path);
    }
    let config = builder.build().map_err(|e: apexstore::LsmError| {
        io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
    })?;

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
    println!("   Prefix Compression: {}", prefix_compression);
    println!();

    let engine = match LsmEngine::new_from_config(
        &config,
        apexstore::storage::cache::GlobalBlockCache::new(64, 4096),
    ) {
        Ok(engine) => engine,
        Err(e) => {
            eprintln!("❌ Error initializing LSM Engine: {}", e);
            eprintln!(
                "Preserve the database and its key files. Verify encryption configuration; legacy zero-key data requires migrate-encryption."
            );
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                e.to_string() as String,
            ));
        }
    };

    println!("✓ Engine initialized successfully!\n");

    let engine = Arc::new(engine);

    // Register default frontmatter schema on startup
    {
        let note_engine = apexstore::notes::NoteEngine::new(engine.clone());
        let _ = apexstore::notes::validate::register_default_schema(&note_engine);
    }

    apexstore::api::start_server(engine, server_config)
        .await
        .map_err(|e: io::Error| e)
}
