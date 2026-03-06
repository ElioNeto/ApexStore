# ApexStore Quick Start Guide

## 🚀 Running ApexStore

ApexStore provides two main interfaces:

### 1. 🖥️ CLI REPL Interface (Interactive)

For development, testing, and interactive data exploration.

```bash
# Start the interactive CLI
cargo run --bin cli

# Or in release mode (faster)
cargo run --release --bin cli
```

**Features:**
- Interactive REPL (Read-Eval-Print Loop)
- All commands: SET, GET, DELETE, SEARCH, STATS, BATCH, etc.
- Perfect for testing and debugging
- See [CLI Guide](docs/CLI_GUIDE.md) for all commands

**Example Session:**
```bash
lsm> SET user:alice Alice Silva
✓ SET 'user:alice' executado com sucesso

lsm> GET user:alice
✓ 'user:alice' = 'Alice Silva'

lsm> SEARCH user: --prefix
✓ 1 registro(s) encontrado(s):
  user:alice = Alice Silva

lsm> STATS ALL
{
  "mem_records": 1,
  "mem_kb": 0,
  ...
}

lsm> exit
👋 Encerrando LSM-Tree CLI...
```

### 2. 🌐 API Server (Production)

For production deployments with REST API access.

```bash
# Start the API server
cargo run --bin apexstore-server

# Or in release mode (recommended for production)
cargo run --release --bin apexstore-server

# With custom configuration
DATA_DIR=./data MEMTABLE_MAX_SIZE=16777216 cargo run --bin apexstore-server
```

**Features:**
- REST API with JSON payloads
- Actix-Web server (high performance)
- Environment-based configuration
- Production-ready with error handling

**Default Server:**
- URL: `http://0.0.0.0:8080`
- Health Check: `curl http://localhost:8080/health`

**Example API Calls:**
```bash
# Insert data
curl -X POST http://localhost:8080/keys \
  -H "Content-Type: application/json" \
  -d '{"key": "user:1", "value": "Alice"}'

# Get data
curl http://localhost:8080/keys/user:1

# Search
curl "http://localhost:8080/keys/search?q=user:&prefix=true"

# Stats
curl http://localhost:8080/stats/all
```

## 📦 Build Commands

### Development Builds (Faster compilation)

```bash
# Build CLI
cargo build --bin cli

# Build server
cargo build --bin apexstore-server

# Build both
cargo build --bins
```

### Release Builds (Optimized performance)

```bash
# Build CLI (optimized)
cargo build --release --bin cli

# Build server (optimized)
cargo build --release --bin apexstore-server

# Run directly after building
./target/release/cli
./target/release/apexstore-server
```

## 🐳 Docker Deployment

### Quick Start with Docker Compose

```bash
# Start server
docker-compose up -d

# View logs
docker-compose logs -f apexstore

# Stop server
docker-compose down
```

### Standalone Docker

```bash
# Build image
docker build -t apexstore:latest .

# Run server
docker run -d \
  --name apexstore-server \
  -p 8080:8080 \
  -v apexstore-data:/data \
  apexstore:latest
```

## ⚙️ Configuration

### CLI Configuration

The CLI uses default settings optimized for development:
- **Data Directory**: `./.lsm_data`
- **MemTable Size**: 4KB (for quick flushes during testing)

You can modify these in `src/cli/mod.rs` or pass environment variables.

### Server Configuration

The server reads configuration from environment variables:

```bash
# Create .env file
cp .env.example .env

# Edit configuration
nano .env

# Start with custom config
source .env
cargo run --bin apexstore-server
```

**Key Variables:**
```bash
DATA_DIR=./.lsm_data
MEMTABLE_MAX_SIZE=16777216      # 16MB
BLOCK_CACHE_SIZE_MB=64
BLOCK_SIZE=4096
HOST=0.0.0.0
PORT=8080
```

See [Configuration Guide](docs/CONFIGURATION.md) for all options.

## 🔍 Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_builder_basic

# Run with output
cargo test -- --nocapture

# Check code quality
cargo clippy -- -D warnings

# Format code
cargo fmt
```

## ❓ Common Issues

### Issue: `cargo run` doesn't start server

**Problem:** Running `cargo run` without `--bin` uses the default binary which just exits.

**Solution:**
```bash
# ❌ Wrong (uses default binary)
cargo run

# ✅ Correct (specify binary)
cargo run --bin apexstore-server  # For API server
cargo run --bin cli               # For CLI REPL
```

### Issue: "Address already in use"

**Problem:** Port 8080 is already occupied.

**Solution 1:** Stop the other process using port 8080
```bash
# Find process
lsof -i :8080

# Kill process
kill <PID>
```

**Solution 2:** Use a different port
```bash
PORT=8081 cargo run --bin apexstore-server
```

### Issue: "Failed to load SSTable" or "Corrupted data"

**Problem:** Incompatible SSTable format from previous version or corrupted data.

**Solution:** Remove old data files
```bash
# Backup first (optional)
cp -r .lsm_data .lsm_data.backup

# Remove old data
rm -rf .lsm_data/*.sst
rm -rf .lsm_data/wal.log

# Restart server/CLI
cargo run --bin apexstore-server
```

### Issue: CLI commands not recognized

**Problem:** Typo or incorrect command syntax.

**Solution:** Use `HELP` command
```bash
lsm> HELP
# Shows all available commands with syntax
```

## 📚 Documentation

- [README.md](README.md) - Project overview and architecture
- [CLI Guide](docs/CLI_GUIDE.md) - Complete CLI reference
- [Configuration Guide](docs/CONFIGURATION.md) - All configuration options
- [API Documentation](docs/API.md) - REST API endpoints
- [Contributing Guide](docs/CONTRIBUTING.md) - Development guidelines

## 🎯 Next Steps

### For Development:
1. Start CLI: `cargo run --bin cli`
2. Try commands: `SET`, `GET`, `SEARCH`, `STATS ALL`
3. Import sample data: `BATCH SET examples/batch_data.txt`
4. Explore: See [CLI Guide](docs/CLI_GUIDE.md)

### For Production:
1. Configure: Copy and edit `.env.example`
2. Build: `cargo build --release --bin apexstore-server`
3. Deploy: Use Docker or systemd service
4. Monitor: Check `/stats/all` endpoint

## 💡 Tips

- **Use release builds** for production (10x faster)
- **Monitor with `STATS ALL`** in CLI or `/stats/all` in API
- **Use prefix-based keys** (e.g., `user:123`, `product:456`) for efficient searches
- **BATCH SET** for bulk imports (faster than individual SETs)
- **Docker** recommended for production deployments

## 🆘 Need Help?

- 📖 Check the [CLI Guide](docs/CLI_GUIDE.md) for command syntax
- 🐛 [Open an issue](https://github.com/ElioNeto/ApexStore/issues) for bugs
- 💬 [Discussions](https://github.com/ElioNeto/ApexStore/discussions) for questions
- 📧 Email: netoo.elio@hotmail.com

---

**Built with 🦀 Rust and ❤️**
