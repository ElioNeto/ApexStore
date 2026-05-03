 <p align="center">
  <img src="docs/assets/logo.png" width="180" alt="ApexStore Logo">
</p>

<h1 align="center">ApexStore</h1>

<p align="center">
  <strong>High-performance, embedded Key-Value engine built with Rust 🦀</strong>
  <br />
  <em>Implementing LSM-Tree architecture with a focus on SOLID principles, observability, and performance.</em>
</p>

<p align="center">
  <a href="https://elioneto.github.io/ApexStore/"><img src="https://img.shields.io/badge/docs-latest-blue.svg" alt="Documentation"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-1.70%2B-orange.svg" alt="Rust Version"></a>
  <a href="https://github.com/ElioNeto/ApexStore/releases"><img src="https://img.shields.io/github/v/release/ElioNeto/ApexStore" alt="Release"></a>
  <a href="https://www.docker.com/"><img src="https://img.shields.io/badge/docker-ready-blue.svg" alt="Docker"></a>
  <a href="https://github.com/ElioNeto/ApexStore/actions/workflows/pr-validation.yml"><img src="https://github.com/ElioNeto/ApexStore/actions/workflows/pr-validation.yml/badge.svg" alt="CI"></a>
</p>

---

## 🎯 Overview

ApexStore is a modern, Rust-based storage engine designed for write-heavy workloads. It combines the durability of write-ahead logging (WAL) with the efficiency of **Log-Structured Merge-Tree (LSM-Tree)** architecture.

Built from the ground up using **SOLID principles**, it provides a production-grade storage solution that is easy to reason about, test, and maintain, while delivering the performance expected from a systems-level language.

## ⚖️ Why ApexStore?

While industry giants like RocksDB or LevelDB focus on extreme complexity, ApexStore offers:

- **Educational Clarity**: A clean, modular implementation of LSM-Tree that serves as a blueprint for high-performance systems.
- **Strict SOLID Compliance**: Leveraging Rust's ownership model to enforce clear boundaries between MemTable, WAL, and SSTable layers.
- **Observability First**: Built-in real-time metrics for memory, disk usage, and WAL health.
- **Modern Defaults**: Native LZ4 compression, Bloom Filters, and 35+ tunable parameters via environment variables.

## 📊 Performance Benchmarks

> **Run locally:** `cargo bench --all-features` → HTML reports at `target/criterion/`

### 🤖 Latest CI Results

<!-- BENCHMARK_RESULTS_START -->
> \U0001f916 Auto-updated by CI on **2026-05-03 10:55 UTC** — [View run](https://github.com/ElioNeto/ApexStore/actions/runs/25277050616)


**📝 Write**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `key_updates/10k_keys` | 7.9271 ms | 🟠 |
| `delete_operations/10k_keys` | 1.4167 ms | 🟠 |
| `write_single/10` | 163.67 ns | 🟢 |
| `write_single/100` | 165.23 ns | 🟢 |
| `write_single/1024` | 193.75 ns | 🟢 |
| `write_single/10240` | 825.88 ns | 🟢 |
| `write_batch_1000/1000` | 349.36 µs | 🟡 |
| `write_batch_10000/10000` | 6.6851 ms | 🟠 |
| `write_batch_100000/100000` | 197.78 ms | 🔴 |
| `memtable_flush_8/8` | 42.744 ms | 🟠 |
| `sstable_flush/100000` | 195.76 ms | 🔴 |
| `write_size_10_10/10x10` | 162.47 ns | 🟢 |
| `write_size_10_100/10x100` | 166.38 ns | 🟢 |
| `write_size_100_100/100x100` | 171.91 ns | 🟢 |
| `write_size_100_1000/100x1000` | 173.27 ns | 🟢 |
| `write_size_100_10000/100x10000` | 173.90 ns | 🟢 |

**📚 Read**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `read_memtable/1000` | 140.69 µs | 🟡 |
| `read_memtable/10000` | 268.95 µs | 🟡 |
| `read_sstable_cold/1000` | 141.60 µs | 🟡 |
| `read_sstable_cold/10000` | 242.45 µs | 🟡 |
| `read_sstable_warm/1000` | 140.84 µs | 🟡 |
| `read_sstable_warm/10000` | 245.62 µs | 🟡 |
| `read_latency/memtable_1k` | 113.64 µs | 🟡 |
| `read_latency/sstable_cold_1k` | 142.11 µs | 🟡 |

**🔍 Scan**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `scan_sequential/1000` | 142.68 µs | 🟡 |
| `scan_sequential/10000` | 1.7862 ms | 🟠 |
| `full_scan/1000` | 119.25 µs | 🟡 |
| `full_scan/10000` | 1.8443 ms | 🟠 |
| `range_scan_100/100` | 5.4828 ms | 🟠 |
| `range_scan_1000/1000` | 5.7221 ms | 🟠 |
| `prefix_scan_100/100` | 690.19 µs | 🟡 |
| `prefix_scan_1000/1000` | 857.23 µs | 🟡 |
| `iteration_sorted/1000` | 146.39 µs | 🟡 |
| `iteration_sorted/10000` | 1.8767 ms | 🟠 |
| `scan_limit_10/10` | 2.1927 µs | 🟢 |
| `scan_limit_100/100` | 20.588 µs | 🟡 |
| `scan_limit_1000/1000` | 206.27 µs | 🟡 |
| `scan_pagination/10` | 681.54 µs | 🟡 |
| `scan_pagination/100` | 59.108 ms | 🟠 |

**🌐 YCSB**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `ycsb_type_a/10000` | 825.22 µs | 🟡 |
| `ycsb_type_b/10000` | 679.56 µs | 🟡 |
| `ycsb_type_c/10000` | 324.20 µs | 🟡 |
| `ycsb_type_c/100000` | 569.66 µs | 🟡 |

**⚡ Mixed Workload**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `workload_balanced/10000` | 829.04 µs | 🟡 |
| `workload_read_heavy/10000` | 728.83 µs | 🟡 |
| `workload_write_heavy/10000` | 858.06 µs | 🟡 |

**🏗️ SSTable**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `sstable_layer_1/1` | 1.7318 ms | 🟠 |
| `sstable_layer_3/3` | 5.5430 ms | 🟠 |
| `sstable_layer_10/10` | 22.282 ms | 🟠 |
| `many_sstables_10/10` | 227.21 µs | 🟡 |
| `many_sstables_50/50` | 320.21 µs | 🟡 |

**🧵 Bloom Filter**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `bloom_filter/10000` | 2.0605 ms | 🟠 |
| `bloom_filter/100000` | 32.317 ms | 🟠 |

**💾 Cache**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `cache_thrash_16MB/16` | 82.503 µs | 🟡 |
| `cache_thrash_64MB/64` | 78.079 µs | 🟡 |

**🧵 Concurrency**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `concurrent_1_threads/1` | 2.6408 ms | 🟠 |
| `concurrent_2_threads/2` | 2.9277 ms | 🟠 |

**💡 Memory**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `memory_pressure/small_memtable` | 4.9195 ms | 🟠 |

<!-- BENCHMARK_RESULTS_END -->



### 📋 YCSB Mixed Workload — `mixed_bench`

*Measured on **Intel Core i5-9300H @ 2.40GHz**, 16 GB DDR4 2667 MHz, HDD SATA 1TB (v2.1.39) — `cargo bench --bench mixed_bench -- --sample-size 10`*

#### Throughput (operations/second)

| Benchmark | Size | Median | Throughput | Change vs previous |
|-----------|------|--------|------------|---------------------|
| **YCSB Type A** *(50% write / 50% read)* | 10K | 952.83 µs | 1.05 Mops/s | no change |
| **YCSB Type A** *(50% write / 50% read)* | 100K | 2.706 ms | 369.6 Kops/s | ✅ +49% throughput |
| **YCSB Type B** *(5% write / 95% read)* | 10K | 814.90 µs | 1.23 Mops/s | ⚠️ -18.6% throughput |
| **YCSB Type B** *(5% write / 95% read)* | 100K | 1.409 ms | 710.0 Kops/s | ⚠️ -20.1% throughput |
| **YCSB Type C** *(100% read)* | 10K | 334.70 µs | 2.99 Mops/s | ✅ +9.4% throughput |
| **YCSB Type C** *(100% read)* | 100K | 745.36 µs | 1.34 Mops/s | ✅ +12.4% throughput |
| **YCSB Type C** *(100% read)* | 1M | 1.290 ms | 775.0 Kops/s | — *(new)* |

#### Composite Workloads

| Benchmark | Size | Median | Throughput | Change vs previous |
|-----------|------|--------|------------|---------------------|
| **Balanced** *(mixed workload)* | 10K | 1.080 ms | 925.9 Kops/s | ⚠️ -8.7% throughput |
| **Balanced** *(mixed workload)* | 100K | 2.831 ms | 353.2 Kops/s | — *(no baseline)* |
| **Read Heavy** *(read-intensive)* | 10K | 811.91 µs | 1.23 Mops/s | ✅ +15.7% throughput |
| **Read Heavy** *(read-intensive)* | 100K | 1.777 ms | 562.7 Kops/s | — *(no baseline)* |
| **Write Heavy** *(write-intensive)* | 10K | 1.187 ms | 82.3 KiB/s | ⚠️ -6.7% throughput |
| **Write Heavy** *(write-intensive)* | 100K | 3.486 ms | 28.0 KiB/s | — *(no baseline)* |

> **Hardware note:** results above are conservative — measured on HDD SATA (vs. NVMe). On NVMe, expect 2–4× better throughput for I/O-bound operations.

> **Key Insights:**
> - `WAL_SYNC_MODE=async` provides 16x throughput vs fsync (trade durability for speed)
> - Cache hit rate > 80% when `block_cache_size_mb > 256`
> - Bloom filter rejects 99.2% of non-existent key lookups
> - Optimal `memtable_max_size` is 16-32MB for write-heavy workloads

## ✨ Key Features

### 🛠️ Storage Engine
- **MemTable**: In-memory BTreeMap with configurable size limits.
- **Write-Ahead Log (WAL)**: ACID-compliant durability with configurable sync modes.
- **SSTable V2**: Block-based storage with Sparse Indexing and LZ4 Compression.
- **Bloom Filters**: Drastically reduces unnecessary disk I/O for read operations.
- **Crash Recovery**: Automatic WAL replay on startup to ensure zero data loss.

### 🔌 Access Patterns
- **Interactive CLI**: REPL interface for development and debugging.
- **REST API**: Full HTTP API with JSON payloads for microservices.
- **Batch Operations**: Efficient bulk inserts and updates.
- **Search Capabilities**: Prefix and substring search (Optimized iterators coming in v2.0).

## 🏗️ Architecture

The engine follows a modular architecture where each component has a single responsibility:

```mermaid
graph TB
    subgraph "Interface Layer"
        CLI[CLI / REPL]
        API[REST API Server]
    end

    subgraph "Core Domain"
        Engine[LSM Engine]
        MemTable[MemTable<br/>BTreeMap]
        LogRecord[LogRecord<br/>Data Model]
    end

    subgraph "Storage Layer"
        WAL[Write-Ahead Log<br/>Durability]
        SST[SSTable Manager<br/>V2 Format]
        Builder[SSTable Builder<br/>Compression]
    end

    subgraph "Infrastructure"
        Codec[Serialization<br/>Bincode]
        Error[Error Handling]
        Config[Configuration<br/>Environment]
    end

    CLI --> Engine
    API --> Engine
    Engine --> WAL
    Engine --> MemTable
    MemTable -->|Flush| Builder
    Builder --> SST
    Engine -->|Read| MemTable
    Engine -->|Read| SST
    WAL -.->|Recovery| MemTable
    
    Engine --> Config
    SST --> Codec
    Builder --> Codec
    WAL --> Codec

    style Engine fill:#f9a,stroke:#333,stroke-width:3px
    style WAL fill:#9cf,stroke:#333,stroke-width:2px
    style SST fill:#9cf,stroke:#333,stroke-width:2px
```

## 🚀 Quick Start

### Prerequisites
- **Rust 1.70+**: Install via [rustup.rs](https://rustup.rs/)

### Installation & Run
```bash
# Clone and enter
git clone https://github.com/ElioNeto/ApexStore.git && cd ApexStore

# Build and Start REPL
cargo run --release

# Available commands:
# > put user:1 "John Doe"
# > get user:1
# > stats
```

## 🐳 Docker Deployment

Run ApexStore as a standalone API server:

```bash
# Start with Docker Compose
docker-compose up -d

# Manual run with custom config
docker run -d \
  --name apexstore-server \
  -p 8080:8080 \
  -e MEMTABLE_MAX_SIZE=33554432 \
  -v apexstore-data:/data \
  elioneto/apexstore:latest
```

## 🌐 REST API Examples

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/keys` | Insert/Update: `{"key": "k1", "value": "v1"}` |
| `GET` | `/keys/{key}` | Retrieve value |
| `GET` | `/stats/all` | Full telemetry (Memory, Disk, WAL) |

## 📁 Project Structure

```
ApexStore/
├── src/
│   ├── core/      # LSM Engine, MemTable, Domain logic
│   ├── storage/   # WAL, SSTable V2, Block Builder
│   ├── infra/     # Codec, Error Handling, Config
│   ├── api/       # Actix-Web Server & Handlers
│   └── cli/       # REPL Implementation
├── docs/          # Detailed documentation & Architecture
├── tests/         # Integration test suite
└── Dockerfile     # Multi-stage build
```

## 🧪 Testing & Quality

```bash
cargo test                  # Run all tests
cargo clippy -- -D warnings # Linting
cargo fmt                   # Formatting
```

## 🚀 CI/CD & Development Workflow

ApexStore uses **trunk-based development** with automated releases:

```mermaid
graph LR
    A[Feature Branch] -->|Open PR| B[CI Validation]
    B -->|✅ Pass| C[Merge to main]
    C --> D[Auto Release]
    D --> E[v2.1.X]
```

### Development Flow

1. **Create feature branch** from `main`
2. **Open PR** → CI runs `cargo fmt`, `clippy`, `test`, `build`
3. **Merge PR** → Auto-increments version in `Cargo.toml`, creates tag & GitHub release

📖 **Read:** [`MIGRATION_GUIDE.md`](MIGRATION_GUIDE.md) for team workflow  
📂 **Details:** [`.github/workflows/README.md`](.github/workflows/README.md)

## 🗺️ Roadmap

- [x] SSTable V2 with compression & Bloom Filters
- [x] REST API & Feature Flags
- [x] Global Block Cache
- [x] Trunk-based CI/CD with auto-release
- [ ] **v2.2**: Storage iterators for range queries
- [ ] **v2.3**: Concurrent read optimization
- [ ] **v3.0**: Leveled/Tiered Compaction Strategies

## 🤝 Contributing

Contributions are what make the open-source community an amazing place! Please check our [Contributing Guidelines](docs/CONTRIBUTING.md).

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feat/amazing-feature`)
3. Commit your Changes (`git commit -m 'feat: add amazing feature'`)
4. Push to the Branch (`git push origin feat/amazing-feature`)
5. Open a Pull Request to `main`
6. CI will auto-release on merge 🚀

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.

## 📧 Contact

**Elio Neto** - [GitHub](https://github.com/ElioNeto) - netoo.elio@hotmail.com  
**Demo**: [lsm-admin-dev.up.railway.app](https://lsm-admin-dev.up.railway.app/)

## 🌟 Star History

[![Star History Chart](https://api.star-history.com/svg?repos=ElioNeto/ApexStore&type=Date)](https://star-history.com/#ElioNeto/ApexStore&Date)

---
<p align="center">Built with 🦀 Rust and ❤️ for high-performance storage systems</p>
