---
name: rust-development
description: Guidelines for Rust development in the ApexStore project
---

# Rust Development Skill

## Build & Test Commands

```bash
cargo build --release          # production build
cargo test --all-features       # run all tests
cargo clippy --all-targets --all-features -- -D warnings  # lint
cargo fmt --all -- --check      # formatting check
cargo doc --no-deps --all-features  # documentation
cargo audit                     # security audit
cargo bench                     # benchmarks
```

## Project Structure

- `src/lib.rs` — library root, re-exports
- `src/core/` — storage engine (memtable, sstable, compaction, WAL, block_cache, merge_iterator, column_family)
- `src/api/` — HTTP API (actix-web routes, handlers, models)
- `src/cli/` — CLI interface (clap)
- `src/tui/` — Terminal UI (ratatui)
- `benches/` — Criterion benchmarks
- `tests/` — Integration tests

## Code Conventions

- `thiserror` for library errors, `anyhow` for binaries
- `?` operator for error propagation (never `unwrap()` in production)
- Explicit lifetimes when needed, not by default
- Unit tests in same file with `#[cfg(test)]`
- Integration tests in `tests/` directory

## LSM-Tree Architecture

- Writes → Memtable (WAL + skiplist) → flush → SSTable L0
- Compaction merges SSTables across levels
- Bloom filters accelerate point reads
- Block cache (LRU) caches deserialized data blocks
- MergeIterator merges multiple sorted iterators
