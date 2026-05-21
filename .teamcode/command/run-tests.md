---
description: "Run Rust tests across the workspace"
---

Run tests for the ApexStore Rust project.

## Run all tests:
```bash
cargo test --all-features --workspace
```

## Run tests matching a filter:
```bash
cargo test --all-features --workspace $ARGUMENTS
```

## Run tests with output:
```bash
cargo test --all-features --workspace -- --nocapture
```

## Run specific test file:
```bash
cargo test --test $ARGUMENTS
```

## Examples
- `compaction` — Run tests matching "compaction"
- `sstable` — Run tests matching "sstable"
- `tests/integration_test.rs` — Run a specific integration test file

**IMPORTANT**: Always run tests from the repo root with `--workspace` to include all crates.
