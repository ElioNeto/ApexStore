# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] — v2.2 (Hardening) → v2.3 (Bug fixes, Features & Resilience)

### 🐛 Critical Bug Fixes

- **#191** — WAL recovery returns stale value after restart: deduplicate records by key during recovery, keeping only the last occurrence per (column_family, key) pair
- **#190** — Compaction panics with index out of bounds in `pick_compaction()`: added bounds checks in `Compaction::compact()` and `LazyLevelingCompaction::pick_tables()`
- **#189** — `VersionSet::get()` does not check `is_deleted`: treat empty values as tombstones (return None)
- **#188** — Compaction detects tombstones by empty value instead of `is_deleted` flag: documented tombstone-as-empty-value convention
- **#180** — Point reads always miss for data in on-disk SSTables: wired `SstableReader` into `VersionSet::get()` for on-disk reads
- **#182** — Server does not handle SIGTERM: added tokio signal handler calling `engine.close()` before graceful shutdown
- **#185** — Server crashes under 500 concurrent connections: added `HttpServer::max_connections()`, `backlog()`, `workers()` config + IP-based rate limiting middleware
- **#186** — 6 `unwrap()`/`expect()` calls in production code: replaced all with proper error handling via `?` and safe fallbacks

### 🔧 Medium Bug Fixes & Chores

- **#178** — `API_AUTH_ENABLED` has no effect: wired Bearer auth middleware respecting `auth.enabled` flag
- **#179** — CLI has no subcommand to create/manage API tokens: added `token create`, `token list`, `token revoke` subcommands
- **#181** — SSTable count mismatch: added `reconcile_tables()`, disk SSTable discovery, and proper cleanup in compaction
- **#183** — Added `cargo-audit` to CI pipeline for dependency vulnerability scanning
- **#184** — Snapshot restore may lose data: `create_snapshot()` now flushes memtables and writes manifest; `restore_snapshot()` reads manifest and registers SSTables

### ✨ High-Priority Features

- **#192** — Range delete: `delete_range(start, end)` with `RangeTombstone` struct tracked in memtable and compaction
- **#193** — TTL/auto-expiry: `expires_at` field in `LogRecord`, `set_with_ttl()`, expiry checks in get/scan/compaction
- **#195** — Encryption at rest: AES-256-GCM for SSTable blocks (LSMSST04 magic) and WAL frames (V3 format), configurable via `--encrypt-key-file`
- **#196** — ACID transactions: `Transaction` struct with `begin_transaction()`, `commit()`, `rollback()`, buffered writes with atomic WAL application

### 🚀 Features

- **#197** — OpenTelemetry integration: OTLP tracing/metrics exporter with fallback to console
- **#198** — Bulk import/export: streaming JSON/CSV import/export via paginated scans and batched writes
- **#199** — Change Data Capture (CDC): event publisher trait, in-memory collector, webhook publisher
- **#200** — Concurrent compaction: semaphore-based parallel compaction across CFs
- **#201** — Web admin dashboard: dark-themed HTML dashboard with auto-refresh
- **#202** — GraphQL API: `/graphql` endpoint with query/mutation support via async-graphql
- **#203** — Memory-mapped SSTable reads: zero-copy I/O via `memmap2` for cold data
- **#204** — Primary-replica replication: WAL shipping with background task, POST /admin/replicate endpoint
- **#205** — SQL query engine: SELECT/INSERT/DELETE via `sqlparser` crate, accessible via CLI and API

### 💡 Differentiator Features

- **#206** — WebAssembly plugin system: `WasmPlugin` with load/call/unload (feature-gated)
- **#207** — Vector search / embeddings index: cosine similarity search
- **#208** — Time-travel queries: query data as of any point in time via timestamped snapshots
- **#209** — Pub/sub messaging: topic-based broadcast via tokio broadcast channels
- **#210** — Automatic data tiering: hot/warm/cold tiers with auto age-out
- **#211** — Multi-model queries: key-value + document + time-series + graph wrapper
- **#212** — Webhook triggers: register webhooks per key prefix, integrated with CDC
- **#213** — CRDT real-time collaboration: LWW register merge/resolve
- **#214** — Blob/attachment storage: chunked large file storage
- **#215** — Budget-aware queries: cost tracking with spend/remaining/is_exhausted
- **#216** — Policy-as-code access control: OPA-style policies with context matchers
- **#217** — Data diff & two-way sync: diff/sync/resolve between instances
- **#218** — CI/CD integration: test fixture management with seed/reset/generate
- **#219** — JSON Schema validation: per-prefix schema enforcement via jsonschema

### 🛡️ Resilience Features

- **#220** — Circuit breaker: Closed/Open/HalfOpen with configurable thresholds
- **#221** — Health check endpoints: `/health/liveness`, `/health/readiness`, `/health/startup`
- **#222** — Disk space monitoring: preemptive shutdown before ENOSPC
- **#223** — Memory limit enforcement: OOM prevention via configurable max memory
- **#224** — Automatic WAL archiving: rotation to timestamped backups
- **#225** — Data integrity scrubber: background SSTable checksum verification
- **#226** — Graceful degradation modes: Normal/ReadOnly/Degraded with write rejection
- **#227** — Request timeout middleware: per-endpoint configurable timeout (default 30s)
- **#228** — Retry with exponential backoff: jitter, configurable retries/delays
- **#229** — Compaction backpressure: write delay when compaction falls behind
- **#230** — Panic recovery: catch_unwind wrappers for worker threads
- **#231** — Enhanced rate limiting: per-IP tracking, per-endpoint limits, admin endpoint
- **#232** — Resource quotas per tenant: keys/storage/rps limits with per-tenant tracking
- **#233** — Automatic backup scheduling: periodic snapshots with configurable retention
- **#234** — Watchdog thread: monitors WAL latency, compaction progress, memtable fill rate
- **#235** — Idempotency key deduplication: TTL-based response cache
- **#236** — Chaos testing framework: inject latency, disk-full, panic, etc. (feature-gated)

### 🔄 Changed

- **#92** — Renamed `LsmError::Serialization(#[from] bincode::Error)` → `Codec` to match `infra::codec` module name; moved variant history table from `src/infra/error.rs` into `CHANGELOG.md`

---

## [2.1.1] — 2026-03-06

### ✨ Added

#### Docker Support & Deployment

- `docker-compose.yml` for single-command deployment (`docker-compose up -d`)
- Automatic environment variable mapping from `.env` file
- Persistent volume (`apexstore-data`) for data durability
- Built-in health checks with configurable intervals
- Resource limits (CPU, memory) for production deployments
- Restart policy: `unless-stopped`

#### README Improvements

- Comprehensive "🐳 Docker Deployment" section
- Quick Start with Docker Compose
- Standalone Docker commands for custom setups
- Health check instructions and data persistence documentation

### 🔄 Changed

#### Branding Updates

- Renamed binary `lsm-server` → `apexstore-server` in Dockerfile
- Added maintainer, description, version labels to Dockerfile
- Added automated health check via curl
- Updated README project structure to include Docker files

### 📚 Documentation

- Docker deployment options documented (Compose, standalone, native)
- Environment variable configuration guide (35+ parameters)
- Backup and restore procedures
- Port mapping and networking guide

### ⚠️ Known Issues (to be fixed in v2.2)

- WAL `clear()` has a race condition between truncate and reopen file handles (#89)
- `set_batch()` / `delete_batch()` are not atomic — partial failure leaves inconsistent state (#90)
- `std::sync::Mutex` used in `engine.rs` and `wal.rs` despite `parking_lot` being a declared dependency (#91)
- `LsmError` enum has duplicate variants: `KeyNotFound` ≡ `NotFound`; `SerializationFailed` overlaps `Serialization` (#92)
- `LsmEngine` fields are `pub(crate)`, bypassing encapsulation invariants (#93)
- `search_in_block()` uses linear scan — binary search already done for sparse index but not inside blocks (#37)

---

## [2.0.0] — 2026-02-XX

### ✨ Added

#### SSTable V2 Reader (`src/storage/reader.rs`)

- Full `SstableReader` implementation for LSMSST02 format
- Binary search on sparse index via `partition_point()` — O(log N) block lookup
- Bloom filter integration for fast negative lookups (lock-free, immutable)
- Shared `Arc<GlobalBlockCache>` with LRU eviction across all readers
- `parking_lot::Mutex<File>` for thread-safe concurrent file access
- Thread-safety verified: 10-thread concurrent read tests passing
- Magic number validation (`LSMSST03`) on open
- Decompressed size validation after LZ4 block decompression

#### Iterator Infrastructure

- `StorageIterator` trait (`src/storage/iterator.rs`)
- `MemTableIterator` with `new()` (from start) and `new_from(key)` (seekable)
- `SstableIterator` (`src/storage/sst_iterator.rs`) — block-aware, cache-integrated
- Full seek support: position iterator at arbitrary key

#### Engine Integration

- `scan()` — full database scan combining MemTable + all SSTables, newest-first, tombstone-aware
- `search(pattern)` — substring match over all keys
- `search_prefix(prefix)` — prefix match over all keys
- `keys()` — all live keys
- `count()` — total live record count
- `stats_all()` — structured `LsmStats` with per-component sizes
- `set_batch()` / `delete_batch()` — multi-record operations

#### REST API Enhancements

- `GET /scan` — full database scan (JSON)
- `GET /keys/search?q=...&prefix=true` — prefix/substring search
- `GET /stats/all` — structured JSON statistics
- JWT/Bearer token authentication system (`src/api/auth/`)
- Admin endpoints: `POST/GET/DELETE /admin/tokens`
- CORS support via `actix-cors`

#### TUI Interface

- Interactive terminal UI (`src/bin/tui.rs`) using `ratatui` + `crossterm`
- Real-time engine statistics display

#### Documentation

- `CHANGELOG.md` — full history
- `MIGRATION_GUIDE.md` — V1 → V2 migration steps
- `QUICKSTART.md` — 5-minute getting started guide
- `book.toml` + `book/` — mdBook documentation structure

### 🔄 Changed

- Cargo package renamed: `lsm-kv-store` → `apex-store-rs`
- Magic number updated: `LSMSST02` → `LSMSST03` (format revision)
- `LsmEngine::new()` now recovers all SSTables from disk on startup
- SSTables sorted by timestamp descending (newest-first) on load
- WAL recovery integrated into engine initialization

### 📦 Dependencies Added

- `ratatui = "0.29"` — TUI framework
- `crossterm = "0.28"` — cross-platform terminal
- `tui-input = "0.10"` — TUI input handling
- `actix-web-httpauth = "0.8"` — Bearer auth middleware
- `chrono = "0.4"` — timestamp formatting
- `uuid = "1.22"` — token IDs
- `sha2 = "0.10"` + `base64 = "0.22"` — token hashing

---

## [1.3.0] — 2026-02-03

### ✨ Added

#### SSTable Builder with Sparse Index (Task 1.2)

- `src/storage/builder.rs` — complete SSTable V2 builder
- Magic header `LSMSST02` for format versioning
- Block-based storage with automatic overflow handling
- Sparse index: `BlockMeta { first_key, offset, size, uncompressed_size }`
- LZ4 compression via `lz4_flex` (2–4x space savings)
- `MetaBlock` with min/max keys, record count, timestamp, Bloom filter data
- Fixed 8-byte footer with meta block offset for O(1) metadata access
- 4 comprehensive builder tests passing

#### Configuration System (PR #29)

- `dotenvy` for `.env` file support
- `.env.example` with 35+ configurable parameters
- `src/api/config.rs` with `ServerConfig` struct
- 5 performance profiles (stress, high-write, high-read, memory-constrained, balanced)
- Configuration display on server startup
- `docs/CONFIGURATION.md` (500+ lines)

### 🔧 Fixed

- Increased default JSON payload from 2MB to 50MB (configurable via `MAX_JSON_PAYLOAD_SIZE`)
- All Clippy violations resolved
- All compilation warnings removed

---

## [1.2.0-beta] — 2026-01-31

### ♻️ Refactored

- Centralized `LsmConfig` with builder pattern
- SOLID architecture throughout
- Removed Portuguese comments; translated all messages to English
- Fixed `SstableConfig` parameter passing

---

## [1.1.0-alpha] — 2026-01-25

### ✨ Added

- GitHub Actions workflows (develop → release)
- Feature flag management API endpoints
- Docker multi-stage build support
- Enhanced stats retrieval

---

## [1.0.0-alpha] — 2026-01-24

### ✨ Added

- MemTable (BTreeMap) with configurable size limit
- WAL (Write-Ahead Log) for durability
- Automatic flush to SSTables on MemTable full
- SSTable V1 with Bloom Filters
- WAL recovery on startup
- Delete via tombstone
- CLI REPL with interactive commands
- REST API with full CRUD operations
- Batch operations
- Basic search (prefix and substring)

### ⚠️ Known Limitations at v1.0

- No compaction (SSTables grow indefinitely)
- No efficient iterators (full scan for all searches)
- No secondary indexes
- No multi-instance support
- No data integrity checksums

---

**Note**: This CHANGELOG follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) and [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
