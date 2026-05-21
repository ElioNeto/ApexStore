---
name: database
description: Work with the LSM-tree storage engine internals — SSTable V2 format, memtable, WAL, Bloom Filters, compaction, and column families. Use when modifying storage engine code.
---

# Storage Engine

ApexStore is an embedded LSM-tree key-value storage engine.

## Architecture

```
┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   Memtable   │────▶│  SSTable    │────▶│   Levels     │
│ (skiplist +  │     │ (V2 format) │     │ (L0..Ln)     │
│  WAL)        │     │             │     │              │
└──────────────┘     └─────────────┘     └──────────────┘
                            │                    │
                     ┌──────┴──────┐      ┌──────┴──────┐
                     │ Bloom Filter │      │ Compaction  │
                     │ (probabilis- │      │ (merge +    │
                     │  tic skip)   │      │  GC)        │
                     └─────────────┘      └─────────────┘
```

### SSTable V2 Format

```
┌──────────────────────────────────────────┐
│ Magic (4 bytes: "APEX")                  │
│ Version (4 bytes)                        │
│ Bloom Filter (serialized bloomfilter)    │
│ Data Block 0                             │
│ Data Block 1                             │
│ ...                                      │
│ Data Block N                             │
│ Block Index (offset, size, key ranges)   │
│ CRC32 checksum                           │
└──────────────────────────────────────────┘
```

### Key Files

| Component | Path |
|-----------|------|
| SSTable reader/writer | `src/core/sstable/` |
| Memtable | `src/core/memtable.rs` |
| WAL | `src/core/wal.rs` |
| Compaction | `src/core/compaction.rs` |
| Bloom Filter | `src/core/bloom.rs` |
| Block Cache (LRU) | `src/core/cache.rs` |
| MergeIterator | `src/core/iter.rs` |
| Column Family | `src/core/cf.rs` |

### Development

```bash
# Run storage engine tests
cargo test --all-features --workspace

# Run specific tests
cargo test compaction
cargo test sstable
cargo test memtable

# Benchmarks
cargo bench -- --noplot

# Build release
cargo build --release
```

## Conventions

- All core engine code in `src/core/`
- Use `thiserror` for error types in library code
- Snake_case for functions/variables, PascalCase for types
- No `unwrap()` in production — use `?` or explicit error handling
- Write unit tests in `#[cfg(test)]` modules alongside code
- Integration tests in `tests/` directory
