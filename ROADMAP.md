# Roadmap — ApexStore

**Last Updated:** 2026-05-22
**Current Version:** v2.3.0
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

## v2.2 — v2.3 — Mega Release: Bug Fixes, Features & Resilience

### ✅ Completed Deliverables

All 59 issues have been implemented:

- **7 critical bugs** fixed: WAL stale recovery (#191), compaction OOB panic (#190), tombstone handling (#189, #188), SSTable point reads (#180), SIGTERM handling (#182), rate limiting (#185)
- **6 medium bugs/chores**: unwrap/expect removal (#186), snapshot restore (#184), cargo-audit (#183), SSTable count mismatch (#181), CLI tokens (#179), auth wiring (#178)
- **4 high-priority features**: ACID transactions (#196), encryption at rest (#195), TTL/auto-expiry (#193), range delete (#192)
- **9 features**: OpenTelemetry (#197), bulk import/export (#198), CDC (#199), concurrent compaction (#200), web dashboard (#201), GraphQL (#202), mmap reads (#203), replication (#204), SQL engine (#205)
- **14 differentiator features**: WASM plugins (#206), vector search (#207), time-travel (#208), pub/sub (#209), data tiering (#210), multi-model (#211), webhooks (#212), CRDT (#213), blob storage (#214), query budgets (#215), access control (#216), data sync (#217), CI/CD fixtures (#218), schema validation (#219)
- **17 resilience features**: circuit breaker (#220), health checks (#221), disk monitor (#222), memory limits (#223), WAL archiving (#224), scrubber (#225), degradation modes (#226), request timeout (#227), retry/backoff (#228), compaction backpressure (#229), panic recovery (#230), enhanced rate limiting (#231), tenant quotas (#232), backup scheduling (#233), watchdog (#234), idempotency (#235), chaos testing (#236)

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
| **v2.0–v2.1** | **❌** | **✅ Released** | **Reader, Iterator, Cache, Auth, Docker** | **2026-03-06**    |
| **v2.2–v2.3** | **❌** | **✅ Current** | **Mega release: 59 issues (bugs, features, resilience)** | **2026-05-22** |
| v2.4        | ❌    | ⏳ Planned   | Benchmark suite                            | ~1 week after     |
| v3-lts      | ✅    | ⏳ Planned   | Compaction + CRC32 checksums               | 6–10 weeks        |
| v4          | ❌    | ⏳ Planned   | Secondary indexes + posting lists          | 6–8 weeks         |
| v5-lts      | ✅    | ⏳ Planned   | Production-ready indexed queries           | 4–6 weeks         |
| v6-lts      | ✅    | ⏳ Planned   | Multi-instance + per-instance codec        | 6–8 weeks         |
| v7          | ❌    | ⏳ Planned   | Mongo-like document layer                  | TBD               |
| v8-lts      | ✅    | ⏳ Planned   | Backup/restore + admin tooling             | TBD               |

---

**Last Updated:** 2026-03-31
**Current Release:** v2.3.0
**Authors:** ApexStore Team
**License:** MIT
