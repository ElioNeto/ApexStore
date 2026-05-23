//! ApexStore Stress Test — Log Application Simulation
//!
//! Simulates an application writing structured logs into ApexStore:
//! - 50,000 log entries across 5 levels (INFO, WARN, ERROR, DEBUG, TRACE)
//! - Small memtable (64KB) forces frequent flushes → SSTable generation
//! - WAL burst: writes many entries, causing WAL rotation + flush cycles
//! - Hot reads from memtable, cold reads from SSTables
//! - Measures time, memory, disk I/O

use apexstore::core::engine::Engine;
use apexstore::infra::config::LsmConfig;
use apexstore::storage::cache::GlobalBlockCache;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const LOG_COUNT: usize = 50_000;
const SMALL_MEMTABLE: usize = 65_536; // 64KB — forces ~800 flushes
const LEVELS: &[&str] = &["INFO", "WARN", "ERROR", "DEBUG", "TRACE"];

#[allow(dead_code)]
struct Stats {
    label: &'static str,
    duration: Duration,
    hits: usize,
    misses: usize,
}

fn generate_log_entry(i: usize) -> (String, String) {
    let level = LEVELS[i % LEVELS.len()];
    let msg = format!("msg_{:06}", i);
    let trace_id = i % 1000;
    let duration_ms = (i * 7) % 5000;

    let key = format!("log/{}/{:020}/{}", level, i, msg);
    let value = format!(
        r#"{{"level":"{}","msg":"{}","src":"app-server-1","trace_id":"trace_{}","duration_ms":{}}}"#,
        level, msg, trace_id, duration_ms
    );
    (key, value)
}

fn measure_disk_io(dir: &TempDir) -> (u64, u64, usize, usize) {
    // SSTables are stored in <dir>/sstables/
    let sst_dir = dir.path().join("sstables");
    let sst_count = if sst_dir.exists() {
        sst_dir
            .read_dir()
            .map(|e| {
                e.filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().contains(".sst"))
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };
    let wal_count = dir
        .path()
        .read_dir()
        .map(|e| {
            e.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains("wal"))
                .count()
        })
        .unwrap_or(0);
    let total_size = dir_size(dir.path());
    (total_size, 0, wal_count, sst_count)
}

fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += dir_size(&path);
            } else if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

#[test]
fn test_log_simulation_stress() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!(
        "║  ApexStore v{} — Log Simulation Stress Test        ║",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "║  {}                               ║",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    );
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    let dir = TempDir::new()?;
    let db_path = dir.path().to_path_buf();
    println!("─── 1. Setup ───");
    println!("  DB dir:    {:?}", db_path);
    println!("  Records:   {}", LOG_COUNT);
    println!(
        "  Memtable:  {} bytes (forces frequent flushes)",
        SMALL_MEMTABLE
    );

    // ── Build engine with small memtable ─────────────────────────
    let mut config = LsmConfig::default();
    config.core.dir_path = db_path.clone();
    config.core.memtable_max_size = SMALL_MEMTABLE;

    let engine =
        Engine::<Arc<GlobalBlockCache>>::new_from_config(&config, GlobalBlockCache::new(1, 4096))?;

    let mut stats = Vec::new();

    // ── Phase 1: Bulk write ──────────────────────────────────────
    println!("\n─── 2. BULK WRITE ({} log entries) ───", LOG_COUNT);
    println!("  Generating and writing...");

    let write_start = Instant::now();
    for i in 0..LOG_COUNT {
        let (key, value) = generate_log_entry(i);
        engine.set(key.as_bytes().to_vec(), value.as_bytes().to_vec())?;

        // Flush periodically to force SSTable generation
        if (i + 1) % 5_000 == 0 {
            let _ = engine.flush_memtable();
            let elapsed = write_start.elapsed();
            let rate = ((i + 1) as f64) / elapsed.as_secs_f64();
            println!(
                "    {} / {} entries ({:.0} ops/s)...",
                i + 1,
                LOG_COUNT,
                rate
            );
        }
    }
    // Final flush to ensure all data is in SSTables
    let _ = engine.flush_memtable();
    let write_dur = write_start.elapsed();
    let write_rate = LOG_COUNT as f64 / write_dur.as_secs_f64();
    let (disk_size_after, _, wal_count_after, sst_count_after) = measure_disk_io(&dir);
    println!("  Write complete:");
    println!("    Elapsed:    {:.2}s", write_dur.as_secs_f64());
    println!("    Throughput: {:.0} ops/s", write_rate);
    println!(
        "    DB size:    {} bytes ({:.1} MB)",
        disk_size_after,
        disk_size_after as f64 / 1_048_576.0
    );

    // ── Phase 2: Storage analysis ────────────────────────────────
    println!("\n─── 3. STORAGE LAYER ANALYSIS ───");
    println!("  WAL files:     {}", wal_count_after);
    println!("  SSTable files: {}", sst_count_after);
    if sst_count_after > 0 {
        let sst_dir = db_path.join("sstables");
        if sst_dir.exists() {
            for entry in std::fs::read_dir(&sst_dir)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                println!(
                    "    {:>8}  {}",
                    humansize(meta.len()),
                    entry.file_name().to_string_lossy()
                );
            }
        }
    }

    // ── Phase 3: Cold reads (from SSTables — all data now flushed) ────
    println!("\n─── 4. COLD READS (SSTable / Disk) ───");
    println!("  Reading 100 oldest entries (now in SSTables)...");

    let cold_start = Instant::now();
    let mut cold_hits = 0u64;
    let mut cold_misses = 0u64;
    for i in 0..100 {
        let (key, _) = generate_log_entry(i);
        match engine.get(key.as_bytes())? {
            Some(_) => cold_hits += 1,
            None => cold_misses += 1,
        }
    }
    let cold_dur = cold_start.elapsed();
    println!(
        "    Hits:  {}  Miss:  {}  Time: {:.2?} ({:.0} µs/op)",
        cold_hits,
        cold_misses,
        cold_dur,
        cold_dur.as_micros() as f64 / 100.0
    );

    stats.push(Stats {
        label: "cold_read (sstable)",
        duration: cold_dur,
        hits: cold_hits as usize,
        misses: cold_misses as usize,
    });

    // ── Phase 4: Write more data and do hot reads BEFORE flush ──
    println!("\n─── 5. HOT READS (Memtable / RAM) ───");
    println!("  Writing and reading 100 fresh entries without flushing...");

    // Write 100 fresh entries that stay in memtable
    for i in LOG_COUNT..LOG_COUNT + 100 {
        let (key, value) = generate_log_entry(i);
        engine.set(key.as_bytes().to_vec(), value.as_bytes().to_vec())?;
    }

    let hot_start = Instant::now();
    let mut hot_hits = 0u64;
    let mut hot_misses = 0u64;
    for i in LOG_COUNT..LOG_COUNT + 100 {
        let (key, _) = generate_log_entry(i);
        match engine.get(key.as_bytes())? {
            Some(_) => hot_hits += 1,
            None => hot_misses += 1,
        }
    }
    let hot_dur = hot_start.elapsed();
    println!(
        "    Hits:  {}  Miss:  {}  Time: {:.2?} ({:.0} µs/op)",
        hot_hits,
        hot_misses,
        hot_dur,
        hot_dur.as_micros() as f64 / 100.0
    );

    stats.push(Stats {
        label: "hot_read (memtable)",
        duration: hot_dur,
        hits: hot_hits as usize,
        misses: hot_misses as usize,
    });

    // ── Phase 5: Prefix scans — log tailing ─────────────────────
    println!("\n─── 6. PREFIX SCANS (Log Tailing) ───");

    for level in LEVELS {
        let scan_start = Instant::now();
        let (results, _) = engine.search_prefix(&format!("log/{}", level), None, 50)?;
        let scan_dur = scan_start.elapsed();
        println!(
            "  Prefix 'log/{}' (50): {:.2?}, {} results",
            level,
            scan_dur,
            results.len()
        );
    }

    // ── Phase 6: Engine stats ────────────────────────────────────
    println!("\n─── 7. ENGINE STATISTICS ───");
    let engine_stats = engine.stats("default")?;
    println!("  SSTable files:   {}", engine_stats.sst_files);
    println!("  SSTable size:    {} KB", engine_stats.sst_kb);
    println!("  Memtable keys:   {}", engine_stats.mem_records);
    println!("  Memtable size:   {} KB", engine_stats.mem_kb);
    println!("  WAL size:        {} KB", engine_stats.wal_kb);

    // ── Phase 7: Summary ─────────────────────────────────────────
    println!("\n─── 8. SUMMARY ───");
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  STRESS TEST RESULTS                                        ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  Write throughput:  {:>14.0} ops/s                ║",
        write_rate
    );
    println!(
        "║  Write time:        {:>14.2}s                    ║",
        write_dur.as_secs_f64()
    );
    println!(
        "║  DB size:           {:>14} bytes        ║",
        humansize(disk_size_after)
    );
    println!(
        "║  SSTable files:     {:>14}                    ║",
        sst_count_after
    );
    println!(
        "║  WAL files:         {:>14}                    ║",
        wal_count_after
    );
    println!(
        "║  Hot read (mem):    {:>9.2?} ({} hits)      ║",
        hot_dur, hot_hits
    );
    println!(
        "║  Cold read (disk):  {:>9.2?} ({} hits)     ║",
        cold_dur, cold_hits
    );
    println!("╚══════════════════════════════════════════════════════════════╝\n");

    // ── Cleanup ──────────────────────────────────────────────────
    drop(engine);
    drop(dir);
    println!("─── 9. CLEANUP ───");
    println!("  All temporary data removed.\n");

    Ok(())
}

fn humansize(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}
