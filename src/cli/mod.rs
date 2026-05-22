//! ApexStore CLI — command-line interface for the key-value store.
//!
//! Usage:
//!   apexstore-cli --db <PATH> get <key>
//!   apexstore-cli --db <PATH> set <key> <value>
//!   apexstore-cli --db <PATH> delete <key>
//!   apexstore-cli --db <PATH> scan [--prefix <PREFIX>] [--limit <N>]
//!   apexstore-cli --db <PATH> keys [--prefix <PREFIX>] [--limit <N>]
//!   apexstore-cli --db <PATH> count [--prefix <PREFIX>]
//!   apexstore-cli --db <PATH> stats
//!   apexstore-cli --db <PATH> flush
//!   apexstore-cli --db <PATH> compact

use crate::core::engine::{Engine, MAX_SCAN_LIMIT};
use crate::infra::config::LsmConfig;
use crate::storage::cache::GlobalBlockCache;
use clap::Parser;
use std::sync::Arc;

type CliEngine = Engine<Arc<GlobalBlockCache>>;

/// ApexStore CLI — embedded LSM-tree key-value store.
#[derive(Parser, Debug)]
#[command(name = "apexstore-cli", version, about)]
struct Cli {
    /// Path to the database directory
    #[arg(short = 'D', long = "db", default_value = "./apexstore_data")]
    db_path: std::path::PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Parser, Debug)]
enum Command {
    /// Get the value for a key
    Get {
        key: String,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Set a key-value pair
    Set {
        key: String,
        value: String,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Delete a key
    Delete {
        key: String,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Scan keys in range
    Scan {
        /// Lower bound (inclusive)
        #[arg(short, long)]
        lower: Option<String>,
        /// Upper bound (exclusive)
        #[arg(short = 'U', long)]
        upper: Option<String>,
        /// Maximum results
        #[arg(short, long, default_value = "100")]
        limit: usize,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// List keys (optionally by prefix)
    Keys {
        /// Key prefix to filter
        #[arg(short, long)]
        prefix: Option<String>,
        /// Maximum results
        #[arg(short, long, default_value = "100")]
        limit: usize,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Count keys (optionally by prefix)
    Count {
        /// Key prefix to filter
        #[arg(short, long)]
        prefix: Option<String>,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Show database statistics
    Stats,
    /// Flush memtable to SSTable
    Flush,
    /// Trigger compaction
    Compact,
}

pub fn main() -> crate::infra::error::Result<()> {
    let cli = Cli::parse();

    // Build config from CLI args
    let config = LsmConfig::builder().dir_path(cli.db_path).build()?;

    // Open engine with a shared block cache
    let cache = GlobalBlockCache::new(100, 4096);
    let engine = Engine::new_from_config(&config, cache)?;

    match cli.command {
        Command::Get { key, cf } => cmd_get(&engine, &cf, &key),
        Command::Set { key, value, cf } => cmd_set(&engine, &cf, &key, &value),
        Command::Delete { key, cf } => cmd_delete(&engine, &cf, &key),
        Command::Scan {
            lower,
            upper,
            limit,
            cf,
        } => cmd_scan(&engine, &cf, lower.as_deref(), upper.as_deref(), limit),
        Command::Keys { prefix, limit, cf } => cmd_keys(&engine, &cf, prefix.as_deref(), limit),
        Command::Count { prefix, cf } => cmd_count(&engine, &cf, prefix.as_deref()),
        Command::Stats => cmd_stats(&engine),
        Command::Flush => cmd_flush(&engine),
        Command::Compact => cmd_compact(&engine),
    }
}

// ── Command implementations ──────────────────────────────────────────────

fn cmd_get(engine: &CliEngine, cf: &str, key: &str) -> crate::infra::error::Result<()> {
    match engine.get_cf(cf, key.as_bytes())? {
        Some(value) => {
            println!("{}", String::from_utf8_lossy(&value));
        }
        None => {
            println!("(not found)");
        }
    }
    Ok(())
}

fn cmd_set(
    engine: &CliEngine,
    cf: &str,
    key: &str,
    value: &str,
) -> crate::infra::error::Result<()> {
    engine.put_cf(cf, key.as_bytes().to_vec(), value.as_bytes().to_vec())?;
    println!("ok");
    Ok(())
}

fn cmd_delete(engine: &CliEngine, cf: &str, key: &str) -> crate::infra::error::Result<()> {
    engine.delete_cf(cf, key.as_bytes())?;
    println!("ok");
    Ok(())
}

fn cmd_scan(
    engine: &CliEngine,
    cf: &str,
    lower: Option<&str>,
    upper: Option<&str>,
    limit: usize,
) -> crate::infra::error::Result<()> {
    let lower_bytes = lower.map(|s| s.as_bytes());
    let upper_bytes = upper.map(|s| s.as_bytes());
    let results = engine.scan_cf(cf, lower_bytes, upper_bytes, Some(limit))?;
    for (key, value) in &results {
        println!(
            "{} = {}",
            String::from_utf8_lossy(key),
            String::from_utf8_lossy(value)
        );
    }
    if results.is_empty() {
        println!("(no results)");
    }
    Ok(())
}

fn cmd_keys(
    engine: &CliEngine,
    cf: &str,
    prefix: Option<&str>,
    limit: usize,
) -> crate::infra::error::Result<()> {
    match prefix {
        Some(pref) => {
            let (results, _cursor) = engine.search_prefix(pref, None, limit)?;
            for (key, _value) in &results {
                println!("{}", String::from_utf8_lossy(key));
            }
            if results.is_empty() {
                println!("(no keys)");
            }
        }
        None => {
            // Full scan with unbounded range
            let results = engine.scan_cf(
                cf,
                None as Option<&[u8]>,
                None as Option<&[u8]>,
                Some(limit),
            )?;
            for (key, _value) in &results {
                println!("{}", String::from_utf8_lossy(key));
            }
            if results.is_empty() {
                println!("(no keys)");
            }
        }
    }
    Ok(())
}

fn cmd_count(
    engine: &CliEngine,
    _cf: &str,
    prefix: Option<&str>,
) -> crate::infra::error::Result<()> {
    match prefix {
        Some(pref) => {
            let (results, _) = engine.search_prefix(pref, None, MAX_SCAN_LIMIT)?;
            println!("{}", results.len());
        }
        None => {
            let count = engine.count()?;
            println!("{}", count);
        }
    }
    Ok(())
}

fn cmd_stats(engine: &CliEngine) -> crate::infra::error::Result<()> {
    let stats = engine.stats("default")?;
    println!("=== ApexStore Statistics ===");
    println!("  SSTables:       {}", stats.sst_files);
    println!("  SSTable size:   {} KB", stats.sst_kb);
    println!("  Memtable keys:  {}", stats.mem_records);
    println!("  WAL size:       {} KB", stats.wal_kb);
    println!("  Total records:  {}", stats.total_records);
    println!("  Levels reached: {}", stats.max_levels_reached);
    Ok(())
}

fn cmd_flush(engine: &CliEngine) -> crate::infra::error::Result<()> {
    engine.flush_memtable()?;
    println!("ok");
    Ok(())
}

fn cmd_compact(engine: &CliEngine) -> crate::infra::error::Result<()> {
    let results = engine.compact()?;
    for (cf, metrics) in &results {
        println!(
            "compacted {}: {} files, {} bytes read, {} bytes written",
            cf, metrics.files_merged, metrics.bytes_read, metrics.bytes_written
        );
    }
    if results.is_empty() {
        println!("(nothing to compact)");
    }
    Ok(())
}
