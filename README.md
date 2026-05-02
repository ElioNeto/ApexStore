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

> **Run locally:** `cargo bench --all-features` → relatórios HTML em `target/criterion/`

### 🤖 Latest CI Results

<!-- BENCHMARK_RESULTS_START -->
> 🤖 Auto-updated by CI on **2026-05-02 21:45 UTC** — [View run](https://github.com/ElioNeto/ApexStore/actions/runs/25262479514)

| Benchmark | Mediana |
|-----------|--------|
| `ycsb_type_a/10000` | 934.16 µs |
| `ycsb_type_b/10000` | 716.14 µs |
| `ycsb_type_c/10000` | 324.69 µs |
| `ycsb_type_c/100000` | 868.58 µs |
| `workload_balanced/10000` | 908.36 µs |
| `workload_read_heavy/10000` | 742.36 µs |
| `workload_write_heavy/10000` | 924.31 µs |
| `read_memtable/1000` | 140.17 µs |
| `read_memtable/10000` | 268.41 µs |
| `read_sstable_cold/1000` | 137.20 µs |
| `read_sstable_cold/10000` | 235.95 µs |
| `read_sstable_warm/1000` | 137.50 µs |
| `read_sstable_warm/10000` | 238.15 µs |
| `bloom_filter/10000` | 1.9844 ms |
| `bloom_filter/100000` | 31.823 ms |
| `read_latency/memtable_1k` | 111.15 µs |
| `read_latency/sstable_cold_1k` | 137.54 µs |
| `scan_sequential/1000` | 123.31 µs |
| `scan_sequential/10000` | 1.7911 ms |
| `full_scan/1000` | 118.04 µs |
| `full_scan/10000` | 1.8464 ms |
| `range_scan_100/100` | 5.5083 ms |
| `range_scan_1000/1000` | 5.7626 ms |
| `prefix_scan_100/100` | 666.57 µs |
| `prefix_scan_1000/1000` | 885.09 µs |
| `iteration_sorted/1000` | 144.16 µs |
| `iteration_sorted/10000` | 1.8787 ms |
| `scan_limit_10/10` | 2.1654 µs |
| `scan_limit_100/100` | 20.510 µs |
| `scan_limit_1000/1000` | 212.29 µs |
| `scan_pagination/10` | 682.69 µs |
| `scan_pagination/100` | 60.270 ms |
| `sstable_layer_1/1` | 1.7202 ms |
| `sstable_layer_3/3` | 5.5286 ms |
| `sstable_layer_10/10` | 23.566 ms |
| `concurrent_1_threads/1` | 2.6733 ms |
| `concurrent_2_threads/2` | 2.9470 ms |
| `memory_pressure/small_memtable` | 5.6470 ms |
| `many_sstables_10/10` | 244.55 µs |
| `many_sstables_50/50` | 327.28 µs |
| `cache_thrash_16MB/16` | 80.787 µs |
| `cache_thrash_64MB/64` | 83.333 µs |
| `key_updates/10k_keys` | 8.0418 ms |
| `delete_operations/10k_keys` | 1.5414 ms |
| `write_single/10` | 163.27 ns |
| `write_single/100` | 164.35 ns |
| `write_single/1024` | 194.20 ns |
| `write_single/10240` | 825.41 ns |
| `write_batch_1000/1000` | 347.78 µs |
| `write_batch_10000/10000` | 6.7199 ms |
| `write_batch_100000/100000` | 209.45 ms |
| `memtable_flush_8/8` | 46.077 ms |
| `sstable_flush/100000` | 215.15 ms |
| `write_size_10_10/10x10` | 163.10 ns |
| `write_size_10_100/10x100` | 166.76 ns |
| `write_size_100_100/100x100` | 172.34 ns |
| `write_size_100_1000/100x1000` | 172.19 ns |
| `write_size_100_10000/100x10000` | 172.02 ns |

<!-- BENCHMARK_RESULTS_END -->


### 📋 YCSB Mixed Workload — `mixed_bench`

*Medido em **Intel Core i5-9300H @ 2.40GHz**, 16 GB DDR4 2667 MHz, HDD SATA 1TB (v2.1.39) — `cargo bench --bench mixed_bench -- --sample-size 10`*

#### Throughput (operações/segundo)

| Benchmark | Tamanho | Mediana | Throughput | Variação vs anterior |
|-----------|---------|---------|------------|----------------------|
| **YCSB Type A** *(50% write / 50% read)* | 10K | 952.83 µs | 1.05 Melem/s | sem mudança |
| **YCSB Type A** *(50% write / 50% read)* | 100K | 2.706 ms | 369.6 Kelem/s | ✅ +49% throughput |
| **YCSB Type B** *(5% write / 95% read)* | 10K | 814.90 µs | 1.23 Melem/s | ⚠️ -18.6% throughput |
| **YCSB Type B** *(5% write / 95% read)* | 100K | 1.409 ms | 710.0 Kelem/s | ⚠️ -20.1% throughput |
| **YCSB Type C** *(100% read)* | 10K | 334.70 µs | 2.99 Melem/s | ✅ +9.4% throughput |
| **YCSB Type C** *(100% read)* | 100K | 745.36 µs | 1.34 Melem/s | ✅ +12.4% throughput |
| **YCSB Type C** *(100% read)* | 1M | 1.290 ms | 775.0 Kelem/s | — *(novo)* |

#### Workloads compostos

| Benchmark | Tamanho | Mediana | Throughput | Variação vs anterior |
|-----------|---------|---------|------------|----------------------|
| **Balanced** *(mix equilibrado)* | 10K | 1.080 ms | 925.9 Kelem/s | ⚠️ -8.7% throughput |
| **Balanced** *(mix equilibrado)* | 100K | 2.831 ms | 353.2 Kelem/s | — *(sem baseline)* |
| **Read Heavy** *(leitura intensiva)* | 10K | 811.91 µs | 1.23 Melem/s | ✅ +15.7% throughput |
| **Read Heavy** *(leitura intensiva)* | 100K | 1.777 ms | 562.7 Kelem/s | — *(sem baseline)* |
| **Write Heavy** *(escrita intensiva)* | 10K | 1.187 ms | 82.3 KiB/s | ⚠️ -6.7% throughput |
| **Write Heavy** *(escrita intensiva)* | 100K | 3.486 ms | 28.0 KiB/s | — *(sem baseline)* |

> **Nota sobre hardware:** resultados acima são conservadores — medidos em HDD SATA (vs. NVMe). Em NVMe espera-se 2–4× melhor throughput em operações I/O-bound.

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
cargo test                 # Run all tests
cargo clippy -- -D warnings # Linting
cargo fmt                  # Formatting
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
