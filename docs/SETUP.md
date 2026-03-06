# Development Setup Guide

This guide will help you set up a development environment for LSM KV Store.

## 💻 System Requirements

### Minimum Requirements
- **OS**: Linux, macOS, or Windows (with WSL2 recommended)
- **RAM**: 4GB (8GB+ recommended for large workloads)
- **Disk**: 2GB free space
- **CPU**: Any modern CPU (multi-core recommended for testing)

### Recommended Setup
- **OS**: Ubuntu 22.04 LTS or macOS 13+
- **RAM**: 16GB
- **Disk**: 10GB free space (SSD preferred)
- **CPU**: 4+ cores

---

## 🛠️ Installing Prerequisites

### 1. Rust Toolchain

#### Installation

```bash
# Install Rust using rustup (recommended)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Follow the prompts, then reload your shell
source $HOME/.cargo/env
```

#### Verify Installation

```bash
# Check Rust version (should be 1.70+)
rustc --version
# Output: rustc 1.75.0 (or higher)

# Check Cargo version
cargo --version
# Output: cargo 1.75.0 (or higher)
```

#### Additional Components

```bash
# Install Clippy (linter)
rustup component add clippy

# Install rustfmt (formatter)
rustup component add rustfmt

# Install rust-src (for IDE support)
rustup component add rust-src
```

### 2. Git

#### Linux (Ubuntu/Debian)
```bash
sudo apt-get update
sudo apt-get install git
```

#### macOS
```bash
# Using Homebrew
brew install git

# Or install Xcode Command Line Tools
xcode-select --install
```

#### Windows
Download from [git-scm.com](https://git-scm.com/download/win)

#### Verify Installation
```bash
git --version
# Output: git version 2.x.x
```

### 3. Code Editor (Recommended: VS Code)

#### Install VS Code
- Download from [code.visualstudio.com](https://code.visualstudio.com/)
- Or use package manager:
  ```bash
  # Ubuntu/Debian
  sudo snap install code --classic
  
  # macOS
  brew install --cask visual-studio-code
  ```

#### Install Extensions

**Essential**:
1. **rust-analyzer** - Rust language support
   - ID: `rust-lang.rust-analyzer`
   - Features: Auto-completion, go-to-definition, inline errors

2. **CodeLLDB** (optional) - Debugging support
   - ID: `vadimcn.vscode-lldb`

**Recommended**:
3. **Better TOML** - TOML syntax highlighting
   - ID: `bungcip.better-toml`

4. **Error Lens** - Inline error highlighting
   - ID: `usernamehw.errorlens`

5. **crates** - Cargo.toml dependency management
   - ID: `serayuzgur.crates`

#### VS Code Settings

Create `.vscode/settings.json` in project root:

```json
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "rust-analyzer.cargo.features": "all",
  "editor.formatOnSave": true,
  "[rust]": {
    "editor.defaultFormatter": "rust-lang.rust-analyzer"
  }
}
```

---

## 📦 Project Setup

### 1. Clone the Repository

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/lsm-kv-store.git
cd lsm-kv-store

# Add upstream remote
git remote add upstream https://github.com/ElioNeto/lsm-kv-store.git

# Verify remotes
git remote -v
```

### 2. Build the Project

```bash
# Debug build (faster compilation)
cargo build

# Release build (optimized)
cargo build --release

# Build with API feature
cargo build --release --features api
```

**First build may take 5-10 minutes** as Cargo downloads and compiles dependencies.

### 3. Run Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_memtable_insert
```

### 4. Configuration

```bash
# Copy environment template
cp .env.example .env

# Edit configuration (optional)
nano .env
```

**Example Development Configuration** (`.env`):
```bash
# Server
HOST=127.0.0.1
PORT=8080

# LSM Engine
DATA_DIR=.lsm_data_dev
MEMTABLE_MAX_SIZE=2097152  # 2MB for faster flushing in dev
BLOCK_CACHE_SIZE_MB=32

# Logging
RUST_LOG=debug
ENABLE_METRICS=true
```

---

## 🧪 Development Workflow

### Running the CLI

```bash
# Debug mode
cargo run

# Release mode (faster)
cargo run --release
```

**Available REPL Commands**:
```
> help                 # Show available commands
> put key value        # Insert or update key
> get key              # Retrieve value
> delete key           # Delete key (tombstone)
> stats                # Show statistics
> exit                 # Exit REPL
```

### Running the API Server

```bash
# Debug mode
cargo run --features api --bin lsm-server

# Release mode
cargo run --release --features api --bin lsm-server

# With custom port
PORT=3000 cargo run --release --features api --bin lsm-server
```

**Testing the API**:
```bash
# Insert a key
curl -X POST http://localhost:8080/keys \
  -H "Content-Type: application/json" \
  -d '{"key": "user:1", "value": "Alice"}'

# Get a key
curl http://localhost:8080/keys/user:1

# Get statistics
curl http://localhost:8080/stats/all
```

### Code Quality Checks

```bash
# Format code
cargo fmt

# Check formatting (CI mode)
cargo fmt -- --check

# Run Clippy linter
cargo clippy

# Clippy with strict mode (CI mode)
cargo clippy -- -D warnings

# Check for unused dependencies
cargo machete  # Requires: cargo install cargo-machete
```

### Running Benchmarks

```bash
# Install criterion
cargo install cargo-criterion

# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench memtable_insert

# Generate HTML report
open target/criterion/report/index.html
```

---

## 🛠️ Debugging

### Using LLDB (VS Code)

1. Install CodeLLDB extension
2. Create `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "type": "lldb",
      "request": "launch",
      "name": "Debug CLI",
      "cargo": {
        "args": ["build", "--bin=lsm-kv-store"]
      },
      "args": [],
      "cwd": "${workspaceFolder}"
    },
    {
      "type": "lldb",
      "request": "launch",
      "name": "Debug Server",
      "cargo": {
        "args": ["build", "--features", "api", "--bin=lsm-server"]
      },
      "args": [],
      "cwd": "${workspaceFolder}"
    },
    {
      "type": "lldb",
      "request": "launch",
      "name": "Debug Test",
      "cargo": {
        "args": ["test", "--no-run", "--lib"]
      },
      "args": ["test_memtable_insert"],
      "cwd": "${workspaceFolder}"
    }
  ]
}
```

3. Set breakpoints in code
4. Press `F5` to start debugging

### Using `println!` Debugging

```rust
// In your code
pub fn put(&mut self, key: &[u8], value: &str) -> Result<()> {
    println!("[DEBUG] Inserting key: {:?}", String::from_utf8_lossy(key));
    self.wal.append(key, value)?;
    println!("[DEBUG] WAL append successful");
    self.memtable.insert(key, value);
    Ok(())
}
```

Run with:
```bash
cargo run -- 2>&1 | grep DEBUG
```

### Using `RUST_LOG`

```bash
# Set log level
export RUST_LOG=debug

# Or inline
RUST_LOG=trace cargo run

# Filter by module
RUST_LOG=apexstore::core::engine=debug cargo run
```

---

## 📈 Performance Profiling

### CPU Profiling (Linux)

```bash
# Install flamegraph
cargo install flamegraph

# Generate flamegraph
sudo cargo flamegraph --bin lsm-server

# Open flamegraph.svg in browser
firefox flamegraph.svg
```

### Memory Profiling

```bash
# Using valgrind (Linux)
valgrind --leak-check=full --track-origins=yes \
  ./target/debug/lsm-kv-store

# Using heaptrack (Linux)
heaptrack ./target/debug/lsm-kv-store
heaptrack_gui heaptrack.lsm-kv-store.*.gz
```

### Benchmark Profiling

```bash
# Profile specific benchmark
cargo bench --bench memtable_bench -- --profile-time=10

# With flamegraph
cargo flamegraph --bench memtable_bench
```

---

## 🧰 Testing

### Unit Tests

```bash
# Run all unit tests
cargo test --lib

# Run tests in specific module
cargo test --lib core::memtable

# Run single test
cargo test test_memtable_insert
```

### Integration Tests

```bash
# Run all integration tests
cargo test --test '*'

# Run specific integration test file
cargo test --test recovery_test
```

### Test Coverage

```bash
# Install tarpaulin (Linux only)
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out Html

# Open report
firefox tarpaulin-report.html
```

### Stress Testing

```bash
# Create stress test script
cat > stress_test.sh << 'EOF'
#!/bin/bash
for i in {1..10000}; do
  curl -X POST http://localhost:8080/keys \
    -H "Content-Type: application/json" \
    -d "{\"key\": \"key_$i\", \"value\": \"value_$i\"}" &
done
wait
EOF

chmod +x stress_test.sh

# Run stress test
./stress_test.sh
```

---

## 📚 Documentation

### Generating Documentation

```bash
# Generate and open docs
cargo doc --open

# Include private items
cargo doc --document-private-items --open

# Generate for all features
cargo doc --all-features --open
```

### Writing Documentation

All public APIs should have documentation:

```rust
/// Brief description (appears in summary).
///
/// Longer description with more details.
///
/// # Arguments
///
/// * `key` - Description of key parameter
/// * `value` - Description of value parameter
///
/// # Returns
///
/// Description of return value
///
/// # Errors
///
/// Description of possible errors
///
/// # Examples
///
/// ```
/// let engine = LsmEngine::new(config)?;
/// engine.put(b"key", "value")?;
/// ```
///
/// # Panics
///
/// Description of panic conditions (if any)
///
/// # Safety
///
/// Description of safety requirements (for unsafe functions)
pub fn put(&mut self, key: &[u8], value: &str) -> Result<()> {
    // Implementation
}
```

---

## 🐞 Troubleshooting

### Common Issues

#### Issue: Compilation Errors After Update

```bash
# Clean build artifacts
cargo clean

# Update dependencies
cargo update

# Rebuild
cargo build
```

#### Issue: Tests Failing Randomly

```bash
# Run tests serially (not in parallel)
cargo test -- --test-threads=1
```

#### Issue: Port Already in Use

```bash
# Find process using port 8080
lsof -i :8080  # macOS/Linux

# Kill process
kill -9 <PID>

# Or use different port
PORT=3000 cargo run --features api --bin lsm-server
```

#### Issue: Out of Disk Space

```bash
# Clean target directory (safe, can be rebuilt)
rm -rf target/

# Clean cargo cache
cargo cache --autoclean
```

#### Issue: Slow Compilation

```bash
# Use faster linker (Linux)
sudo apt-get install lld
export RUSTFLAGS="-C link-arg=-fuse-ld=lld"

# Or use mold (even faster)
cargo install mold
export RUSTFLAGS="-C link-arg=-fuse-ld=mold"

# Enable incremental compilation (in Cargo.toml)
[profile.dev]
incremental = true
```

---

## 🚀 Next Steps

1. ✅ Read [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines
2. ✅ Explore the codebase structure
3. ✅ Pick an issue to work on
4. ✅ Join discussions on GitHub

---

## 💬 Getting Help

- **Documentation**: Run `cargo doc --open`
- **Issues**: [GitHub Issues](https://github.com/ElioNeto/lsm-kv-store/issues)
- **Discussions**: [GitHub Discussions](https://github.com/ElioNeto/lsm-kv-store/discussions)
- **Email**: netoo.elio@hotmail.com

---

**Happy Coding!** 🦀

*Last updated: February 2026*