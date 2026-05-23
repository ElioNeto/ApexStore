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

use crate::api::auth::token::{ApiToken, Permission};
use crate::api::auth::TokenManager;
use crate::core::engine::{Engine, MAX_SCAN_LIMIT};
use crate::infra::cdc::CdcConfig;
use crate::infra::cicd::{Fixture, FixtureEntry};
use crate::infra::config::LsmConfig;
use crate::infra::sql::{format_sql_result, SqlEngine};
use crate::storage::cache::GlobalBlockCache;
use clap::{Parser, Subcommand};
use std::sync::Arc;

type CliEngine = Engine<Arc<GlobalBlockCache>>;

/// ApexStore CLI — embedded LSM-tree key-value store.
#[derive(Parser, Debug)]
#[command(name = "apexstore-cli", version, about)]
struct Cli {
    /// Path to the database directory
    #[arg(short = 'D', long = "db", default_value = "./apexstore_data")]
    db_path: std::path::PathBuf,

    /// Path to file containing the hex-encoded AES-256 encryption key (64 hex chars).
    /// When provided, enables transparent encryption at rest for SSTables and WAL.
    #[arg(long = "encrypt-key-file")]
    encrypt_key_file: Option<std::path::PathBuf>,

    /// CDC endpoint URL for streaming data changes (e.g. http://localhost:9000/webhook).
    /// When set, CDC is enabled and data mutations are posted as JSON to this endpoint.
    #[arg(long = "cdc-endpoint")]
    cdc_endpoint: Option<String>,

    #[command(subcommand)]
    command: Command,
}

/// Token prefix used for storing API tokens in the engine
const TOKEN_PREFIX: &str = "__token:";
const FIXTURE_PREFIX: &str = "__fixture:";

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
    /// Execute SQL query against the engine
    Sql {
        /// SQL query to execute (e.g. "SELECT * FROM default", "INSERT INTO default (key, value) VALUES ('k', 'v')")
        query: String,
    },
    /// Import key-value pairs from a file
    Import {
        /// File format: "json" or "csv"
        format: String,
        /// Path to the input file (use "-" for stdin)
        file: String,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Export key-value pairs to a file
    Export {
        /// File format: "json" or "csv"
        format: String,
        /// Path to the output file (use "-" for stdout)
        file: String,
        /// Column family (default: "default")
        #[arg(short, long, default_value = "default")]
        cf: String,
    },
    /// Manage API tokens
    #[command(subcommand)]
    Token(TokenCommand),
    /// Manage test fixtures
    #[command(subcommand)]
    Fixture(FixtureCommand),
}

/// Fixture management subcommands
#[derive(Subcommand, Debug)]
enum FixtureCommand {
    /// List registered fixtures
    List,
    /// Load fixture entries into the engine by name
    Load {
        /// Name of the fixture to load
        name: String,
    },
    /// Generate test data and register it as a fixture
    Generate {
        /// Name for the new fixture
        name: String,
        /// Number of entries to generate
        count: u64,
    },
    /// Register a fixture with explicit key=value pairs
    Register {
        /// Name for the new fixture
        name: String,
        /// Comma-separated key=value pairs (e.g. "k1=v1,k2=v2")
        #[arg(short, long, value_delimiter = ',')]
        keys: Vec<String>,
    },
}

/// Token management subcommands
#[derive(Subcommand, Debug)]
enum TokenCommand {
    /// Create a new API token with optional permissions
    Create {
        /// Human-readable name for the token
        name: String,
        /// Permissions to grant (default: read). Options: read, write, delete, admin
        #[arg(short, long, default_values = &["read"])]
        permissions: Vec<String>,
    },
    /// List all API tokens
    List,
    /// Revoke (delete) an API token by its ID
    Revoke {
        /// Token ID to revoke
        id: String,
    },
}

pub fn main() -> crate::infra::error::Result<()> {
    let cli = Cli::parse();

    // Build config from CLI args
    let mut builder = LsmConfig::builder().dir_path(cli.db_path);
    if let Some(key_path) = cli.encrypt_key_file {
        let key_str = key_path.to_string_lossy().to_string();
        builder = builder
            .encryption_enabled(true)
            .encryption_key_path(key_str);
    }
    let config = builder.build()?;

    // Open engine with a shared block cache
    let cache = GlobalBlockCache::new(100, 4096);
    let engine = Engine::new_from_config(&config, cache)?;

    // Configure CDC if an endpoint was provided
    if let Some(endpoint) = &cli.cdc_endpoint {
        let cdc_config = CdcConfig::with_endpoint(endpoint.clone());
        engine.set_cdc(cdc_config);
        tracing::info!(target: "apexstore::cli", "CDC enabled, endpoint: {}", endpoint);
    }

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
        Command::Sql { query } => cmd_sql(&engine, &query),
        Command::Import { format, file, cf } => cmd_import(&engine, &format, &file, &cf),
        Command::Export { format, file, cf } => cmd_export(&engine, &format, &file, &cf),
        Command::Token(sub) => cmd_token(&engine, sub),
        Command::Fixture(sub) => cmd_fixture(&engine, sub),
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

fn cmd_sql(engine: &CliEngine, query: &str) -> crate::infra::error::Result<()> {
    let sql_engine = SqlEngine::new(engine);
    let result = sql_engine.execute(query)?;
    let output = format_sql_result(&result);
    print!("{}", output);
    Ok(())
}

// ── Import / Export command implementations ──────────────────────────────────

/// Handle `import` subcommand.
fn cmd_import(
    engine: &CliEngine,
    format: &str,
    file: &str,
    cf: &str,
) -> crate::infra::error::Result<()> {
    use crate::infra::bulk_io;

    let start = std::time::Instant::now();

    // Progress callback that prints a simple progress line
    let progress: Option<bulk_io::ProgressFn> = Some(Box::new(|current, total| {
        if total > 0 {
            eprint!("\rImported: {} / {} records", current, total);
        } else {
            eprint!("\rImported: {} records", current);
        }
    }));

    match format.to_lowercase().as_str() {
        "json" => {
            if file == "-" {
                bulk_io::import_json(engine, std::io::stdin(), Some(cf), progress)?;
            } else {
                let f = std::fs::File::open(file)?;
                let reader = std::io::BufReader::new(f);
                bulk_io::import_json(engine, reader, Some(cf), progress)?;
            }
        }
        "csv" => {
            if file == "-" {
                bulk_io::import_csv(engine, std::io::stdin(), Some(cf), progress)?;
            } else {
                let f = std::fs::File::open(file)?;
                let reader = std::io::BufReader::new(f);
                bulk_io::import_csv(engine, reader, Some(cf), progress)?;
            }
        }
        other => {
            return Err(crate::infra::error::LsmError::InvalidArgument(format!(
                "Unsupported import format: '{}'. Use 'json' or 'csv'.",
                other
            )));
        }
    }

    let elapsed = start.elapsed();
    eprintln!(); // newline after progress
    println!("Import completed in {:.2}s", elapsed.as_secs_f64());
    Ok(())
}

/// Handle `export` subcommand.
fn cmd_export(
    engine: &CliEngine,
    format: &str,
    file: &str,
    cf: &str,
) -> crate::infra::error::Result<()> {
    use crate::infra::bulk_io;

    let start = std::time::Instant::now();

    let progress: Option<bulk_io::ProgressFn> = Some(Box::new(|current, total| {
        if total > 0 {
            eprint!("\rExported: {} / {} records", current, total);
        } else {
            eprint!("\rExported: {} records", current);
        }
    }));

    match format.to_lowercase().as_str() {
        "json" => {
            if file == "-" {
                bulk_io::export_json(engine, &mut std::io::stdout(), Some(cf), progress)?;
            } else {
                let f = std::fs::File::create(file)?;
                let mut writer = std::io::BufWriter::new(f);
                bulk_io::export_json(engine, &mut writer, Some(cf), progress)?;
            }
        }
        "csv" => {
            if file == "-" {
                bulk_io::export_csv(engine, &mut std::io::stdout(), Some(cf), progress)?;
            } else {
                let f = std::fs::File::create(file)?;
                let mut writer = std::io::BufWriter::new(f);
                bulk_io::export_csv(engine, &mut writer, Some(cf), progress)?;
            }
        }
        other => {
            return Err(crate::infra::error::LsmError::InvalidArgument(format!(
                "Unsupported export format: '{}'. Use 'json' or 'csv'.",
                other
            )));
        }
    }

    let elapsed = start.elapsed();
    eprintln!(); // newline after progress
    println!("Export completed in {:.2}s", elapsed.as_secs_f64());
    Ok(())
}

// ── Token command implementations ──────────────────────────────────────────

/// Load all tokens from the engine (persisted under `__token:*` keys).
fn load_tokens_from_engine(engine: &CliEngine) -> crate::infra::error::Result<Vec<ApiToken>> {
    let (results, _cursor) = engine.search_prefix(TOKEN_PREFIX, None, MAX_SCAN_LIMIT)?;
    let mut tokens = Vec::new();
    for (_key, value) in &results {
        if let Ok(token) = serde_json::from_slice::<ApiToken>(value) {
            tokens.push(token);
        }
    }
    Ok(tokens)
}

/// Save a list of tokens to the engine (replaces all existing token entries).
fn save_tokens_to_engine(
    engine: &CliEngine,
    tokens: &[ApiToken],
) -> crate::infra::error::Result<()> {
    // Remove all existing __token:* keys
    let existing = load_tokens_from_engine(engine)?;
    for token in &existing {
        let key = format!("{}{}", TOKEN_PREFIX, token.id);
        engine.delete_cf("default", key.as_bytes())?;
    }
    // Write all tokens
    for token in tokens {
        let key = format!("{}{}", TOKEN_PREFIX, token.id);
        let value = serde_json::to_vec(token)?;
        engine.put_cf("default", key.as_bytes().to_vec(), value)?;
    }
    Ok(())
}

fn cmd_token(engine: &CliEngine, sub: TokenCommand) -> crate::infra::error::Result<()> {
    match sub {
        TokenCommand::Create { name, permissions } => {
            let parsed_perms: Vec<Permission> = permissions
                .iter()
                .map(|p| {
                    p.parse::<Permission>()
                        .map_err(|e| crate::infra::error::LsmError::InvalidArgument(e.to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let manager = TokenManager::new();
            let (raw_token, api_token) = manager
                .create_token(name, None, parsed_perms)
                .map_err(|e| crate::infra::error::LsmError::InvalidArgument(e.to_string()))?;

            // Persist the token
            let mut tokens = load_tokens_from_engine(engine)?;
            tokens.push(api_token.clone());
            save_tokens_to_engine(engine, &tokens)?;

            println!("Token created successfully!");
            println!("  ID:    {}", api_token.id);
            println!("  Name:  {}", api_token.name);
            println!("  Token: {}", raw_token);
            println!();
            println!("⚠  Store this token securely. It will not be shown again.");
            Ok(())
        }
        TokenCommand::List => {
            let tokens = load_tokens_from_engine(engine)?;
            if tokens.is_empty() {
                println!("No tokens found.");
                return Ok(());
            }
            println!(
                "{:<38} {:<20} {:<10} {:<20}",
                "ID", "Name", "Perms", "Created"
            );
            println!("{}", "-".repeat(90));
            for token in &tokens {
                let perms_str: Vec<String> = token
                    .permissions
                    .iter()
                    .map(|p| format!("{:?}", p))
                    .collect();
                let epoch_secs = token.created_at / 1_000_000_000;
                // Format as a simple date string
                let created = chrono::DateTime::from_timestamp(epoch_secs as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| epoch_secs.to_string());
                println!(
                    "{:<38} {:<20} {:<10} {:<20}",
                    token.id,
                    token.name,
                    perms_str.join(","),
                    created,
                );
            }
            Ok(())
        }
        TokenCommand::Revoke { id } => {
            let mut tokens = load_tokens_from_engine(engine)?;
            let before = tokens.len();
            tokens.retain(|t| t.id != id);
            if tokens.len() == before {
                println!("Token not found: {}", id);
                return Ok(());
            }
            save_tokens_to_engine(engine, &tokens)?;
            println!("Token revoked: {}", id);
            Ok(())
        }
    }
}

// ── Fixture prefix ──────────────────────────────────────────────────────────

/// Store a fixture definition in the engine under `__fixture:{name}`.
fn save_fixture_to_engine(
    engine: &CliEngine,
    fixture: &Fixture,
) -> crate::infra::error::Result<()> {
    let key = format!("{}{}", FIXTURE_PREFIX, fixture.name);
    let value = serde_json::to_vec(fixture)?;
    engine.put_cf("default", key.as_bytes().to_vec(), value)?;
    Ok(())
}

/// Load all fixture definitions from the engine.
fn load_fixtures_from_engine(engine: &CliEngine) -> crate::infra::error::Result<Vec<Fixture>> {
    let (results, _cursor) = engine.search_prefix(FIXTURE_PREFIX, None, MAX_SCAN_LIMIT)?;
    let mut fixtures = Vec::new();
    for (_key, value) in &results {
        if let Ok(fixture) = serde_json::from_slice::<Fixture>(value) {
            fixtures.push(fixture);
        }
    }
    Ok(fixtures)
}

// ── Fixture command implementations ─────────────────────────────────────────

fn cmd_fixture(engine: &CliEngine, sub: FixtureCommand) -> crate::infra::error::Result<()> {
    match sub {
        FixtureCommand::List => {
            let fixtures = load_fixtures_from_engine(engine)?;
            if fixtures.is_empty() {
                println!("No fixtures registered.");
                return Ok(());
            }
            println!("{:<30} {:<10}", "Name", "Entries");
            println!("{}", "-".repeat(42));
            for fixture in &fixtures {
                println!("{:<30} {:<10}", fixture.name, fixture.entries.len());
            }
            Ok(())
        }
        FixtureCommand::Load { name } => {
            let fixtures = load_fixtures_from_engine(engine)?;
            let fixture = fixtures.iter().find(|f| f.name == name);
            match fixture {
                Some(f) => {
                    for entry in &f.entries {
                        engine.put_cf(
                            "default",
                            entry.key.clone(),
                            entry.value.clone(),
                        )?;
                    }
                    println!(
                        "Fixture '{}' loaded ({} entries).",
                        name,
                        f.entries.len()
                    );
                }
                None => {
                    println!("Fixture '{}' not found.", name);
                }
            }
            Ok(())
        }
        FixtureCommand::Generate { name, count } => {
            let mut entries = Vec::with_capacity(count as usize);
            for i in 0..count {
                entries.push(FixtureEntry {
                    key: format!("fixture_{}_{}", name, i).into_bytes(),
                    value: format!("value_{}", i).into_bytes(),
                });
            }
            let fixture = Fixture {
                name: name.clone(),
                entries,
            };
            save_fixture_to_engine(engine, &fixture)?;
            println!(
                "Fixture '{}' generated and registered ({} entries).",
                name, count
            );
            Ok(())
        }
        FixtureCommand::Register { name, keys } => {
            let mut entries = Vec::with_capacity(keys.len());
            for pair in &keys {
                if let Some(eq_pos) = pair.find('=') {
                    let k = pair.as_bytes()[..eq_pos].to_vec();
                    let v = pair.as_bytes()[eq_pos + 1..].to_vec();
                    entries.push(FixtureEntry { key: k, value: v });
                } else {
                    return Err(crate::infra::error::LsmError::InvalidArgument(
                        format!("Invalid key=value pair: '{}'. Use format key=value.", pair),
                    ));
                }
            }
            let fixture = Fixture {
                name: name.clone(),
                entries,
            };
            save_fixture_to_engine(engine, &fixture)?;
            println!(
                "Fixture '{}' registered ({} entries).",
                name,
                keys.len()
            );
            Ok(())
        }
    }
}
