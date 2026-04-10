# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased] — v2.2 (Hardening)

### 🔧 Fixes Planned

- **#89** — WAL `clear()` race condition: replace two-handle truncate pattern with `set_len(0)` + `seek(Start(0))` on the existing fd to eliminate crash-recovery data loss window
- **#90** — `set_batch()` / `delete_batch()` non-atomic: rewrite to use single WAL pass + single memtable lock acquisition per batch
- **#91** — Migrate `std::sync::Mutex` → `parking_lot::Mutex`/`RwLock` in `engine.rs` and `wal.rs`; upgrade `sstables` to `RwLock` for concurrent read access
- **#92** — Remove duplicate `LsmError` variants (`KeyNotFound` ≡ `NotFound`, `SerializationFailed` / `DeserializationFailed` overlap with `Serialization`)
- **#93** — Encapsulate `LsmEngine` fields (remove `pub(crate)` on all struct fields; add private fields + accessor methods)
- **#37** — Replace linear in-block scan with `binary_search_by()` in `search_in_block()` (sparse index binary search already done)

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
