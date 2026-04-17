use crate::{LsmConfig, LsmEngine};
use std::io::{self, Write};
use std::path::PathBuf;

const DEFAULT_SCAN_LIMIT: usize = 1000;
const MAX_SCAN_LIMIT: usize = 10000;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_level(true)
        .init();

    println!("    ___                     _____ __                 ");
    println!("   /   |  ____  ___  _  __ / ___// /_____  _____ ___ ");
    println!(r"  / /| | / __ \/ _ \| |/_/ \__ \/ __/ __ \/ ___// _ \");
    println!(r" / ___ |/ /_/ /  __/>  <   ___/ / /_/ /_/ / /  /  __/");
    println!(r"/_/  |_/ .___/\___/_/|_|  /____/\__/\____/_/   \___/ ");
    println!("      /_/   High-Performance LSM-Tree Engine\n");

    // Configuration
    let config = LsmConfig::builder()
        .dir_path(PathBuf::from("./.lsm_data"))
        .memtable_max_size(4 * 1024) // 4KB for tests
        .build()?;

    println!("📂 Data directory: {}", config.core.dir_path.display());

    println!("Initializing engine...");
    let engine = LsmEngine::new(config)?;
    println!("✓ Engine initialized successfully!\n");

    print_help();
    println!();

    // REPL Loop
    loop {
        print!("ApexStore (CLI): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let parts: Vec<&str> = input.splitn(3, ' ').collect();
        let command = parts[0].to_uppercase();

        match command.as_str() {
            "SET" => {
                if parts.len() < 3 {
                    println!("❌ Usage: SET <key> <value>");
                    continue;
                }
                let key = parts[1].to_string();
                let value = parts[2].as_bytes().to_vec();
                //let value = parts[2..].join(" ").as_bytes().to_vec();

                match engine.set(key.clone(), value) {
                    Ok(_) => println!("✓ SET '{}' executed successfully", key),
                    Err(e) => println!("❌ Error: {}", e),
                }
            }

            "GET" => {
                if parts.len() < 2 {
                    println!("❌ Usage: GET <key>");
                    continue;
                }
                let key = parts[1];

                match engine.get(key) {
                    Ok(Some(value)) => {
                        let value_str = String::from_utf8_lossy(&value);
                        println!("✓ '{}' = '{}'", key, value_str);
                    }
                    Ok(None) => println!("⚠ Key '{}' not found", key),
                    Err(e) => println!("❌ Error: {}", e),
                }
            }

            "DELETE" | "DEL" => {
                if parts.len() < 2 {
                    println!("❌ Usage: DELETE <key>");
                    continue;
                }
                let key = parts[1].to_string();

                match engine.delete(key.clone()) {
                    Ok(_) => println!("✓ DELETE '{}' executed (tombstone created)", key),
                    Err(e) => println!("❌ Error: {}", e),
                }
            }

            "STATS" => {
                // Check if "ALL" parameter is provided
                if parts.len() > 1 && parts[1].to_uppercase() == "ALL" {
                    match engine.stats_all() {
                        Ok(stats) => match serde_json::to_string_pretty(&stats) {
                            Ok(json) => println!("{}", json),
                            Err(e) => println!("❌ Error serializing JSON: {}", e),
                        },
                        Err(e) => println!("❌ Error: {}", e),
                    }
                } else {
                    println!("{}", engine.stats());
                }
            }

            "SEARCH" => {
                if parts.len() < 2 {
                    println!("❌ Usage: SEARCH <query> [--prefix]");
                    continue;
                }

                let query = parts[1];
                let prefix_mode = parts.len() > 2 && parts[2] == "--prefix";

                let results = if prefix_mode {
                    engine.search_prefix_legacy(query)
                } else {
                    engine.search(query)
                };

                match results {
                    Ok(records) => {
                        if records.is_empty() {
                            println!("⚠ No records found");
                        } else {
                            println!("✓ {} record(s) found:\n", records.len());
                            for (key, value) in records {
                                let value_str = String::from_utf8_lossy(&value);
                                println!("  {} = {}", key, value_str);
                            }
                        }
                    }
                    Err(e) => println!("❌ Error: {}", e),
                }
            }

            "HELP" | "?" => {
                print_help();
            }

            "CLEAR" => {
                print!("\x1B[2J\x1B[1;1H"); // Clear screen ANSI code
                println!("    ___                     _____ __                 ");
                println!("   /   |  ____  ___  _  __ / ___// /_____  _____ ___ ");
                println!(r"  / /| | / __ \/ _ \| |/_/ \__ \/ __/ __ \/ ___// _ \");
                println!(r" / ___ |/ /_/ /  __/>  <   ___/ / /_/ /_/ / /  /  __/");
                println!(r"/_/  |_/ .___/\___/_/|_|  /____/\__/\____/_/   \___/ ");
                println!("      /_/   High-Performance LSM-Tree Engine\n");
            }

            "EXIT" | "QUIT" | "Q" => {
                println!("👋 Closing ApexStore... See you later");
                break;
            }

            "DEMO" => {
                run_demo(&engine)?;
            }

            "BATCH" => {
                if parts.len() >= 3 && parts[1].to_uppercase() == "SET" {
                    // BATCH SET <file>
                    let file_path = parts[2];
                    println!("Importing from {}...", file_path);

                    match std::fs::read_to_string(file_path) {
                        Ok(content) => {
                            let mut count = 0;
                            let mut errors = 0;

                            for (line_num, line) in content.lines().enumerate() {
                                let line = line.trim();
                                if line.is_empty() || line.starts_with('#') {
                                    continue; // Skip empty lines and comments
                                }

                                if let Some((key, value)) = line.split_once('=') {
                                    let key = key.trim();
                                    let value = value.trim();

                                    match engine.set(key.to_string(), value.as_bytes().to_vec()) {
                                        Ok(_) => count += 1,
                                        Err(e) => {
                                            println!("⚠ Error on line {}: {}", line_num + 1, e);
                                            errors += 1;
                                        }
                                    }
                                } else {
                                    println!(
                                        "⚠ Invalid line {} (expected format: key=value)",
                                        line_num + 1
                                    );
                                    errors += 1;
                                }
                            }

                            println!("✓ {} record(s) imported", count);
                            if errors > 0 {
                                println!("⚠ {} error(s) found", errors);
                            }
                        }
                        Err(e) => println!("❌ Error reading file: {}", e),
                    }
                } else if parts.len() >= 2 {
                    // BATCH <count> (existing functionality)
                    let count: usize = match parts[1].parse() {
                        Ok(n) => n,
                        Err(_) => {
                            println!("❌ Invalid count");
                            continue;
                        }
                    };

                    println!("Inserting {} records...", count);
                    let start = std::time::Instant::now();

                    for i in 0..count {
                        let key = format!("batch:{}", i);
                        let value = format!("value_{}", i).into_bytes();
                        engine.set(key, value)?;
                    }

                    let elapsed = start.elapsed();
                    println!("✓ {} records inserted in {:.2?}", count, elapsed);
                    println!("  Rate: {:.0} ops/s", count as f64 / elapsed.as_secs_f64());
                } else {
                    println!("❌ Usage: BATCH <count> | BATCH SET <file>");
                }
            }

            "SCAN" => {
                // SCAN [start_key] [end_key] [limit]
                let start_key = if parts.len() > 1 && !parts[1].is_empty() {
                    Some(parts[1].to_string())
                } else {
                    None
                };

                let end_key = if parts.len() > 2 && !parts[2].is_empty() {
                    Some(parts[2].to_string())
                } else {
                    None
                };

                let limit: usize = if parts.len() > 3 && !parts[3].is_empty() {
                    match parts[3].parse() {
                        Ok(n) if n > 0 => n,
                        Ok(_) => {
                            println!("❌ Usage: SCAN [start_key] [end_key] [limit]");
                            println!("   limit must be greater than 0");
                            continue;
                        }
                        Err(_) => {
                            println!("❌ Usage: SCAN [start_key] [end_key] [limit]");
                            continue;
                        }
                    }
                } else {
                    DEFAULT_SCAN_LIMIT
                };

                // Validate limit
                if limit > MAX_SCAN_LIMIT {
                    println!("❌ Limit {} exceeds maximum allowed limit {}", limit, MAX_SCAN_LIMIT);
                    continue;
                }

                // Validate start < end
                if let (Some(ref s), Some(ref e)) = (&start_key, &end_key) {
                    if s >= e {
                        println!("❌ start_key '{}' must be less than end_key '{}'", s, e);
                        continue;
                    }
                }

                println!("Scanning range [{:?}, {:?}) with limit {}...", start_key, end_key, limit);

                // Fetch first page
                let mut fetched = 0;
                let mut last_key: Option<String> = None;
                let mut has_more = true;

                while has_more {
                    match engine.scan_range(
                        start_key.as_ref().map(|s| s.as_str()),
                        end_key.as_ref().map(|e| e.as_str()),
                        limit,
                    ) {
                        Ok((records, next_cursor)) => {
                            if records.is_empty() {
                                if fetched == 0 {
                                    println!("⚠ No records found in range");
                                }
                                break;
                            }

                            for (key, value) in &records {
                                let value_str = String::from_utf8_lossy(value);
                                println!("  {} = {}", key, value_str);
                                fetched += 1;
                            }

                            last_key = next_cursor.clone();

                            if records.len() < limit {
                                has_more = false;
                            } else {
                                // Continue with pagination
                                if last_key.is_none() {
                                    has_more = false;
                                } else {
                                    // For pagination, we use the next page starting after last_key
                                    // The range scan doesn't support cursor-based pagination directly,
                                    // so we use end_key for the first query, then continue
                                    // For simplicity, just stop here for now
                                    has_more = false;
                                }
                            }
                        }
                        Err(e) => {
                            println!("❌ Error: {}", e);
                            break;
                        }
                    }
                }

                if fetched == 0 {
                    println!("⚠ No records found");
                } else {
                    println!("✓ {} total record(s) found", fetched);
                }
            }

            "ALL" => {
                println!("Listing all records...\n");
                match engine.scan() {
                    Ok(records) => {
                        if records.is_empty() {
                            println!("⚠ Database is empty");
                        } else {
                            println!("┌─────────────────────────────────────────────────┐");
                            println!("│  Key                  │  Value                 │");
                            println!("├─────────────────────────────────────────────────┤");

                            for (key, value) in records {
                                let value_str = String::from_utf8_lossy(&value);
                                let key_display = if key.len() > 20 {
                                    format!("{}...", &key[..17])
                                } else {
                                    key.clone()
                                };
                                let value_display = if value_str.len() > 20 {
                                    format!("{}...", &value_str[..17])
                                } else {
                                    value_str.to_string()
                                };
                                println!("│  {:<20} │  {:<20} │", key_display, value_display);
                            }

                            println!("└─────────────────────────────────────────────────┘");
                        }
                    }
                    Err(e) => println!("❌ Error scanning: {}", e),
                }
            }

            "KEYS" => {
                // KEYS [prefix] [limit]
                let prefix = if parts.len() > 1 && !parts[1].is_empty() {
                    Some(parts[1].to_string())
                } else {
                    None
                };

                let limit: usize = if parts.len() > 2 && !parts[2].is_empty() {
                    match parts[2].parse() {
                        Ok(n) if n > 0 => n,
                        Ok(_) => {
                            println!("❌ Usage: KEYS [prefix] [limit]");
                            println!("   limit must be greater than 0");
                            continue;
                        }
                        Err(_) => {
                            println!("❌ Usage: KEYS [prefix] [limit]");
                            continue;
                        }
                    }
                } else {
                    DEFAULT_SCAN_LIMIT
                };

                if limit > MAX_SCAN_LIMIT {
                    println!("❌ Limit {} exceeds maximum allowed limit {}", limit, MAX_SCAN_LIMIT);
                    continue;
                }

                if let Some(ref p) = prefix {
                    println!("Searching keys with prefix '{}' (limit {})...", p, limit);

                    // Use search_prefix with pagination
                    let mut fetched = 0;
                    let mut cursor: Option<String> = None;

                    loop {
                        match engine.search_prefix(p.as_str(), cursor.as_deref(), limit) {
                            Ok((records, next_cursor)) => {
                                if records.is_empty() {
                                    break;
                                }

                                for (key, _value) in &records {
                                    println!("  {}", key);
                                    fetched += 1;
                                }

                                if next_cursor.is_none() || records.len() < limit {
                                    break;
                                }
                                cursor = next_cursor;
                            }
                            Err(e) => {
                                println!("❌ Error: {}", e);
                                break;
                            }
                        }
                    }

                    if fetched == 0 {
                        println!("⚠ No keys found");
                    } else {
                        println!("✓ {} total key(s) found", fetched);
                    }
                } else {
                    println!("Listing all keys (limit {})...", limit);

                    // List all keys with pagination
                    let mut fetched = 0;
                    let mut cursor: Option<String> = None;

                    loop {
                        match engine.scan_range(None, None, limit) {
                            Ok((records, next_cursor)) => {
                                if records.is_empty() {
                                    break;
                                }

                                for (key, _value) in &records {
                                    println!("  {}", key);
                                    fetched += 1;
                                }

                                if next_cursor.is_none() || records.len() < limit {
                                    break;
                                }
                                cursor = next_cursor;
                            }
                            Err(e) => {
                                println!("❌ Error: {}", e);
                                break;
                            }
                        }
                    }

                    if fetched == 0 {
                        println!("⚠ No keys found");
                    } else {
                        println!("✓ {} total key(s) found", fetched);
                    }
                }
            },

            "COUNT" => match engine.count() {
                Ok(count) => println!("✓ Total active records: {}", count),
                Err(e) => println!("❌ Error: {}", e),
            },

            "PREFIX" => {
                // PREFIX <prefix> [limit]
                if parts.len() < 2 {
                    println!("❌ Usage: PREFIX <prefix> [limit]");
                    continue;
                }

                let prefix = parts[1].to_string();

                let limit: usize = if parts.len() > 2 && !parts[2].is_empty() {
                    match parts[2].parse() {
                        Ok(n) if n > 0 => n,
                        Ok(_) => {
                            println!("❌ Usage: PREFIX <prefix> [limit]");
                            println!("   limit must be greater than 0");
                            continue;
                        }
                        Err(_) => {
                            println!("❌ Usage: PREFIX <prefix> [limit]");
                            continue;
                        }
                    }
                } else {
                    DEFAULT_SCAN_LIMIT
                };

                if limit > MAX_SCAN_LIMIT {
                    println!("❌ Limit {} exceeds maximum allowed limit {}", limit, MAX_SCAN_LIMIT);
                    continue;
                }

                println!("Searching keys with prefix '{}' (limit {})...", prefix, limit);

                // Use search_prefix with pagination
                let mut fetched = 0;
                let mut cursor: Option<String> = None;

                loop {
                    match engine.search_prefix(prefix.as_str(), cursor.as_deref(), limit) {
                        Ok((records, next_cursor)) => {
                            if records.is_empty() {
                                break;
                            }

                            for (key, value) in &records {
                                let value_str = String::from_utf8_lossy(value);
                                println!("  {} = {}", key, value_str);
                                fetched += 1;
                            }

                            if next_cursor.is_none() || records.len() < limit {
                                break;
                            }
                            cursor = next_cursor;
                        }
                        Err(e) => {
                            println!("❌ Error: {}", e);
                            break;
                        }
                    }
                }

                if fetched == 0 {
                    println!("⚠ No keys found");
                } else {
                    println!("✓ {} total key(s) found", fetched);
                }
            },

            _ => {
                println!("❌ Unknown command: '{}'", command);
                println!("   Type HELP to see available commands");
            }
        }
    }

    Ok(())
}

fn print_help() {
    println!("Available commands:");
    println!("  SET <key> <value>         - Insert or update a key-value pair");
    println!("  GET <key>                 - Retrieve the value of a key");
    println!("  DELETE <key>              - Remove a key (creates tombstone)");
    println!("  SEARCH <query> [--prefix] - Search records (optionally by prefix)");
    println!("  SCAN [start] [end] [limit]- Scan range of keys (lexicographic)");
    println!("  KEYS [prefix] [limit]     - List keys (optionally filtered by prefix)");
    println!("  PREFIX <prefix> [limit]   - Shortcut: list keys with prefix");
    println!("  ALL                       - List all database records");
    println!("  COUNT                     - Count active records");
    println!("  STATS [ALL]               - Display statistics (basic or detailed)");
    println!("  BATCH <count>             - Insert N test records");
    println!("  BATCH SET <file>          - Import records from file");
    println!("  DEMO                      - Run feature demonstration");
    println!("  CLEAR                     - Clear the screen");
    println!("  HELP or ?                 - Display this help");
    println!("  EXIT, QUIT or Q           - Exit the program");
}

fn run_demo(engine: &LsmEngine) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║                  AUTOMATIC DEMO                       ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    println!("1. Inserting sample data...");
    engine.set("user:alice".to_string(), b"Alice Silva".to_vec())?;
    engine.set("user:bob".to_string(), b"Bob Santos".to_vec())?;
    engine.set("user:charlie".to_string(), b"Charlie Costa".to_vec())?;
    println!("   ✓ 3 users inserted\n");

    println!("2. Reading data...");
    if let Some(v) = engine.get("user:alice")? {
        println!("   user:alice = {}", String::from_utf8_lossy(&v));
    }
    if let Some(v) = engine.get("user:bob")? {
        println!("   user:bob = {}", String::from_utf8_lossy(&v));
    }
    println!();

    println!("3. Updating user:alice...");
    engine.set("user:alice".to_string(), b"Alice Silva Santos".to_vec())?;
    if let Some(v) = engine.get("user:alice")? {
        println!("   user:alice = {} (updated)", String::from_utf8_lossy(&v));
    }
    println!();

    println!("4. Deleting user:bob...");
    engine.delete("user:bob".to_string())?;
    match engine.get("user:bob")? {
        Some(_) => println!("   ❌ Error: still exists"),
        None => println!("   ✓ user:bob deleted successfully"),
    }
    println!();

    println!("5. Forcing multiple writes to trigger flush...");
    for i in 0..10 {
        engine.set(
            format!("product:{}", i),
            format!("Product {} - Long description to force automatic flush", i).into_bytes(),
        )?;
    }
    println!("   ✓ 10 products inserted\n");

    println!("6. Testing new commands...");
    println!("   - SEARCH user:");
    match engine.search("user:") {
        Ok(results) => println!("     Found {} records", results.len()),
        Err(e) => println!("     Error: {}", e),
    }

    println!("   - SEARCH user: --prefix");
    match engine.search_prefix_legacy("user:") {
        Ok(results) => println!("     Found {} records", results.len()),
        Err(e) => println!("     Error: {}", e),
    }
    println!();

    println!("7. Final statistics (basic):");
    println!("{}", engine.stats());

    println!("\n8. Detailed statistics:");
    match engine.stats_all() {
        Ok(stats) => {
            if let Ok(json) = serde_json::to_string_pretty(&stats) {
                println!("{}", json);
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    println!("\n╔═══════════════════════════════════════════════════════╗");
    println!("║               DEMO COMPLETED                          ║");
    println!("╚═══════════════════════════════════════════════════════╝\n");

    Ok(())
}
