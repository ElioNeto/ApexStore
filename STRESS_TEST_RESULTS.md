# ApexStore v2.1.57 — Stress Test Results

**Date:** 2026-05-22 16:24 UTC  
**Branch:** `test/stress-log-simulation`  
**Test file:** `tests/stress_log_simulation.rs`

---

## Test Scenario: Log Application Simulation

Simulated an application writing 50,000 structured log entries (INFO, WARN, ERROR, DEBUG, TRACE) with a 64KB memtable to force frequent flushes.

### 1. Write Performance

| Metric | Value |
|--------|-------|
| Total entries | 50,000 |
| Entry size | ~85 bytes (key ~40 bytes + JSON value ~45 bytes) |
| Total data | ~4.25 MB (raw), 2.8 MB (on disk after flush) |
| Elapsed | **13.20 seconds** |
| Throughput | **3,788 ops/s** |
| Flushes triggered | ~10 (every 5,000 entries) |

### 2. Storage Layer

| Metric | Value |
|--------|-------|
| SSTable files generated | **19** |
| SSTable total size | ~2.8 MB |
| WAL files | 1 (per-CF) |
| WAL size | ~19 KB (cleared between flushes) |

### 3. Read Performance

| Read Type | Source | Hits | Time | µs/op |
|-----------|--------|------|------|-------|
| **Hot** | Memtable (RAM) | 100/100 ✅ | 215 µs | **~2 µs** |
| **Cold** | SSTable (disk) | 0/100 ⚠️ | 503 µs | ~5 µs |

**Note:** Cold SSTable reads return 0 hits because `VersionSet::get()` only reads from in-memory `table.data` (BTreeMap). On-disk SSTable data is accessible only through `SstableReader`, which is not wired into the point-read path. This is a known architectural gap.

### 4. Prefix Scans (Log Tailing)

| Prefix | Time | Results |
|--------|------|---------|
| `log/INFO` | 3.94 ms | 50 |
| `log/WARN` | 7.11 ms | 50 |
| `log/ERROR` | 1.50 ms | 50 |
| `log/DEBUG` | 0.10 ms | 50 |
| `log/TRACE` | 4.36 ms | 50 |

### 5. Resource Usage

| Metric | Value |
|--------|-------|
| Mem RSS (idle) | ~9.8 MB |
| DB on disk | 2.8 MB |
| SSTable files | 19 |
| I/O write | ~165 KB (test run) |
| I/O read | 0 bytes |

### 6. Engine Statistics (post-test)

| Metric | Value |
|--------|-------|
| SSTable files tracked | 5 |
| SSTable size (tracked) | 843 KB |
| Memtable keys | 100 (freshly written for hot test) |
| WAL size | 19 KB |

---

## Key Observations

1. **Write throughput scales well** — 3,788 ops/s with per-CF WAL + batch fsync
2. **WAL burst handling** — WAL is cleared asynchronously per CF flush, no unbounded growth
3. **Memtable reads are fast** — ~2 µs/op (BTreeMap lookup)
4. **Cold reads miss** — SSTable data is not indexed for point reads; only flushes + scans work from disk
5. **SSTable generation** — 19 SSTables created for 50K entries (average ~2,600 entries/SSTable)
6. **Prefix scans are functional** — 0.1–7 ms depending on match distribution

## Issues Found (New)

- **Cold reads from disk return 0 hits** — `VersionSet::get()` only checks in-memory `table.data`. On-disk SSTable data requires `SstableReader` which is not called.
- **SSTable count mismatch** — Engine stats report 5 SSTable files, but 19 exist on disk. The engine's `VersionSet` only tracks tables added via `add_table()` during flush, some of which were likely already merged during compaction.
