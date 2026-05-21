---
name: testing
description: Testing patterns and infrastructure for the ApexStore Rust project. Use when writing, fixing, or debugging tests. Covers cargo test, criterion benchmarks, and integration tests.
---

# Testing

This project uses **cargo test** for unit and integration tests, and **criterion** for benchmarks.

## Running Tests

```bash
# Run all tests
cargo test --all-features --workspace

# Run tests matching a filter
cargo test compaction
cargo test sstable

# Run with output
cargo test --all-features --workspace -- --nocapture

# Run specific integration test
cargo test --test integration_test

# Run benchmarks
cargo bench -- --noplot

# Run specific benchmark
cargo bench --bench write_bench -- --noplot
```

## Test Patterns

### Unit Tests
Unit tests live in `#[cfg(test)]` modules within source files:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_get() {
        let mut db = create_test_db();
        db.put("key", "value").unwrap();
        assert_eq!(db.get("key").unwrap(), Some("value".into()));
    }

    #[test]
    fn test_key_not_found() {
        let db = create_test_db();
        assert_eq!(db.get("nonexistent").unwrap(), None);
    }
}
```

### Integration Tests
Integration tests go in `tests/` directory:

```rust
// tests/integration_test.rs
use apexstore::*;

#[test]
fn test_full_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(dir.path()).unwrap();
    db.put("k1", "v1").unwrap();
    db.put("k2", "v2").unwrap();
    db.flush().unwrap();
    assert_eq!(db.get("k1").unwrap(), Some("v1".into()));
}
```

### Benchmarks
Benchmarks use criterion and live in `benches/`:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_write(c: &mut Criterion) {
    c.bench_function("write_1k", |b| {
        b.iter(|| {
            let mut db = create_test_db();
            for i in 0..1000 {
                db.put(format!("k{}", i), "v").unwrap();
            }
        })
    });
}
```

## Key Test Locations

| Type | Location |
|------|----------|
| Unit tests | `src/**/*.rs` (inline `#[cfg(test)]`) |
| Integration tests | `tests/*.rs` |
| Benchmarks | `benches/*.rs` |
| Benchmark utilities | `benches/utils.rs` |

## Guidelines

- Unit tests alongside code in `#[cfg(test)]` modules
- Integration tests in `tests/` directory
- Use `tempfile` for temporary directories
- Use `#[should_panic]` for expected failures
- Prefer `?` operator with `anyhow::Result` in tests
- Benchmarks must use `criterion` harness (`harness = false` in Cargo.toml)
- Always run `cargo clippy` and `cargo fmt` alongside tests
