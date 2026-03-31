# Roadmap — ApexStore

**Last Updated:** 2026-03-31
**Current Version:** v2.1.1
**Base Storage Model:** `key: String -> value: Vec<u8>` (LSM-Tree)
**Objective:** Evolve the project through versioned releases, adding **compaction**, **range iterators**, **secondary indexes**, and multi-instance support.

---

## Version Convention

- **Regular versions** (e.g., v2.2, v2.3): Evolutionary releases with new features
- **LTS versions** (e.g., v3-lts, v5-lts): Stable versions, production-ready, focused on compatibility and reliability

---

## ✅ Released Versions

### v1.0.0 — Alpha (2026-01-24)
- MemTable (BTreeMap), WAL, basic SSTable V1 with Bloom Filters
- CLI REPL, REST API (Actix-Web), batch operations
- Single-instance design

### v1.1.0 — Alpha (2026-01-25)
- GitHub Actions workflows (develop → release)
- Feature flag management endpoints
- Docker multi-stage build
- Enhanced statistics

### v1.2.0 — Beta (2026-01-31)
- Centralized `LsmConfig` + builder pattern
- SOLID architecture refactoring
- Removed duplicate configs, translated all messages to English

### v1.3.0 — Stable (2026-02-03)
- **SSTable V2 format (LSMSST02)**: block-based storage, LZ4 compression, sparse index
- **`SstableBuilder`**: complete implementation with Bloom filter generation
- **Configuration System**: 35+ parameters, `.env` support, 5 performance profiles
- `docs/CONFIGURATION.md` (500+ lines)

### v2.0.0 → v2.1.1 — Current (2026-03-06)
- **`SstableReader`**: full V2 reader with binary search on sparse index, Bloom filter, shared block cache
- **Engine Integration**: `scan()`, `search()`, `search_prefix()`, `keys()`, `count()` fully integrated
- **Iterator Infrastructure**: `StorageIterator` trait, `MemTableIterator`, `SstableIterator` (`src/storage/iterator.rs`, `src/storage/sst_iterator.rs`)
- **Global Block Cache**: shared `Arc<GlobalBlockCache>` across all SSTable readers with LRU eviction
- **Concurrent Reader**: `SstableReader` is thread-safe via `parking_lot::Mutex<File>` + immutable metadata
- **REST API + Auth**: JWT/token auth, admin endpoints, health check, Docker Compose deployment
- **TUI**: interactive terminal interface (`ratatui` + `crossterm`)
- **CHANGELOG**, **MIGRATION_GUIDE**, **QUICKSTART** documentation

---

## v2.2 — Bug Fixes & Hardening (Next — ~2 weeks)

### Objective
Fix known correctness and durability bugs identified in the v2.1.1 audit. No new features — stability first.

### Deliverables

#### 🔴 Critical Fixes

- [ ] **#89** — Fix WAL `clear()` race condition between truncate and reopen
  - Replace two-handle pattern with `set_len(0)` + `seek(Start(0))` on the existing fd
  - Eliminates crash-recovery data loss window

- [ ] **#90** — Fix `set_batch()` / `delete_batch()` non-atomic behavior
  - Single WAL pass + single memtable lock acquisition for all items
  - Prevents partial-write inconsistency on error mid-batch

#### 🟡 Refactoring

- [ ] **#91** — Migrate `std::sync::Mutex` → `parking_lot::Mutex` / `RwLock` in `engine.rs` and `wal.rs`
  - `sstables` upgraded to `RwLock` for concurrent reads
  - ~30% lock overhead reduction on hot paths

- [ ] **#92** — Clean up duplicate `LsmError` variants (`KeyNotFound` vs `NotFound`, serialization overlap)

- [ ] **#93** — Remove `pub(crate)` field exposure from `LsmEngine`; add private fields with accessor methods

#### 🟢 Optimization

- [ ] **#37** — Replace linear in-block scan with `binary_search_by()` in `search_in_block()`
  - Sparse index binary search already done; this completes the lookup chain to O(log n) inside the block

### Release Criteria
- All critical bugs (#89, #90) fixed and tested
- Zero `std::sync` usage in hot paths
- All existing tests passing

---

## v2.3 — Range Scan API & Pagination (~2 weeks after v2.2)

### Objective
Make the API production-usable for large datasets by eliminating full-scan materializations.

### Deliverables

- [ ] **#24** — `GET /scan?start_key=...&end_key=...&limit=N` with cursor-based pagination
- [ ] **#24** — `GET /keys/search?q=...&prefix=true&limit=N&cursor=...`
- [ ] Engine: `scan_range(start: &str, end: &str)` leveraging `BTreeMap::range()` + SSTable iterator
- [ ] CLI: `SCAN [start] [end]` and `PREFIX <prefix>` commands
- [ ] Default limit of 1000 records per response (configurable)
- [ ] Response includes `next_cursor` when result set is truncated

### Release Criteria
- `GET /scan` on a 10M-key database returns in < 100ms for limit=100
- Full scan no longer materializes all records in memory

---

## v2.4 — Benchmark Suite (~1 week after v2.3)

### Objective
Replace informal performance claims with real `criterion` benchmarks.

### Deliverables

- [ ] **#48** — Create `benches/` directory with:
  - `write_bench.rs`: single write, batch write, WAL overhead
  - `read_bench.rs`: MemTable hit, SSTable cold/warm cache, Bloom filter
  - `mixed_bench.rs`: YCSB-style workloads A/B/C/D/F
  - `scan_bench.rs`: full scan, range scan, prefix scan
- [ ] CI integration: run benchmarks on `main` push, alert on >10% regression
- [ ] Update README with real measured numbers
- [ ] Create `docs/PERFORMANCE.md`

---

## v3-lts — Compaction 🏷️ (~6–10 weeks)

### Objective
Make the system sustainable for continuous operation. Without compaction, SSTable count grows unboundedly, reads degrade, and disk space is never reclaimed.

### Deliverables

#### Core Compaction Infrastructure

- [ ] **#47** — `src/storage/compaction/` module:
  - `CompactionStrategy` trait
  - `CompactionPicker` — selects SSTables to merge
  - `CompactionExecutor` — performs merge via MergeIterator, writes new SSTable, atomically swaps old files
  - `BackgroundScheduler` — tokio task that triggers compaction on threshold breach

#### Strategy 1: Size-Tiered Compaction (STC)
- Group SSTables into size buckets
- Merge when bucket reaches 4+ files
- Low write amplification (~2–3x), good for write-heavy workloads

#### Strategy 2: Leveled Compaction (LC)
- L0 (new flushes, may overlap) → L1..Ln (non-overlapping, sorted by key range)
- Industry standard (RocksDB, LevelDB)
- Better read amplification, higher write amplification (~10–20x)

#### Tombstone GC
- Remove tombstones permanently during compaction when no older SSTables reference the key

#### Admin API
- `POST /admin/compact` — trigger manual compaction
- `GET /admin/compaction/status` — monitor progress

#### Checksums
- [ ] **#25** — CRC32 per block (append 4 bytes to block encoding)
- Verify on read; return `LsmError::CorruptedData` on mismatch

### LTS Criteria
- SSTable count stabilizes over 72h of continuous writes
- Read latency p99 does not degrade after 1M writes
- Space reclamation works correctly (tombstones removed)
- All correctness tests passing under concurrent load

**Expected Timeline:** 6–10 weeks after v2.4

---

## v4 — Secondary Indexes (Posting Lists)

### Objective
Enable value-based queries without full scan.

### Deliverables

- Index Registry: `indexes.toml` config, multiple extractor types (`raw`, `json_path`, `bson_path`)
- Posting Lists stored as LSM keys: `idx:{index}:{term}:blk:{N} -> [pk1, pk2, ...]`
- `POST /query` endpoint with mandatory index usage (no scan fallback)
- On-write index maintenance; lazy delete; compaction integration

**Expected Timeline:** 6–8 weeks after v3-lts

---

## v5-lts — Production Indexed Queries 🏷️

### Objective
Make indexed queries reliable and operable in production.

### Deliverables

- Posting list intersection (AND/OR/NOT)
- Skip pointers for optimization
- Stable cursors: `(term, block_id, offset)`
- Query timeouts, result limits, `max_postings_scanned` protection
- Index management endpoints (`GET/POST/DELETE /indexes`)

**Expected Timeline:** 4–6 weeks after v4

---

## v6-lts — Multi-Instance + Per-Instance Codec 🏷️

### Objective
Run multiple independent engine instances on the same server.

### Deliverables

- `lsm.toml` with `[[instance]]` definitions
- Per-instance routing: `GET /db/{instance}/keys/{key}`
- Codec layer: `raw`, `json`, `bson`
- Complete data isolation per instance

**Expected Timeline:** 6–8 weeks after v5-lts

---

## Version Summary

| Version     | LTS? | Status      | Main Milestone                             | Timeline          |
| :---------- | :--- | :---------- | :----------------------------------------- | :---------------- |
| v1.0–v1.3   | ❌    | ✅ Released  | SSTable V2, Config, CLI, API               | Done              |
| **v2.0–v2.1** | **❌** | **✅ Current** | **Reader, Iterator, Cache, Auth, Docker** | **2026-03-06**    |
| v2.2        | ❌    | 🔧 Next      | Bug fixes: WAL race, batch atomicity, locks | ~2 weeks         |
| v2.3        | ❌    | ⏳ Planned   | Range scan API + pagination                | ~2 weeks after    |
| v2.4        | ❌    | ⏳ Planned   | Benchmark suite                            | ~1 week after     |
| v3-lts      | ✅    | ⏳ Planned   | Compaction + CRC32 checksums               | 6–10 weeks        |
| v4          | ❌    | ⏳ Planned   | Secondary indexes + posting lists          | 6–8 weeks         |
| v5-lts      | ✅    | ⏳ Planned   | Production-ready indexed queries           | 4–6 weeks         |
| v6-lts      | ✅    | ⏳ Planned   | Multi-instance + per-instance codec        | 6–8 weeks         |
| v7          | ❌    | ⏳ Planned   | Mongo-like document layer                  | TBD               |
| v8-lts      | ✅    | ⏳ Planned   | Backup/restore + admin tooling             | TBD               |

---

**Last Updated:** 2026-03-31
**Current Release:** v2.1.1
**Authors:** ApexStore Team
**License:** MIT
