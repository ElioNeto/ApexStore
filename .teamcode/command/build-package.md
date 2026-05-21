---
description: "Build the ApexStore Rust project"
---

Build the ApexStore Rust storage engine.

## Usage

### Debug build:
```bash
cargo build
```

### Release build:
```bash
cargo build --release
```

### Specific binary:
```bash
cargo build --bin apexstore-server
cargo build --bin apexstore-cli
cargo build --bin apexstore-tui
```

### Check compilation (faster than build):
```bash
cargo check --all-features
```

$ARGUMENTS
