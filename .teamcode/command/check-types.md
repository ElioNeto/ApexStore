---
description: "Run Rust type checking and linting"
---

Run Rust type checks and lints.

## Cargo check (fast type checking, no codegen):
```bash
cargo check --all-features
```

## Clippy lint check:
```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Rustfmt formatting check:
```bash
cargo fmt --all -- --check
```

## Full pre-PR validation:
```bash
cargo check --all-features && cargo clippy --all-targets --all-features -- -D warnings && cargo fmt --all -- --check
```

$ARGUMENTS
