# Performance Documentation

This document contains performance benchmarks for the ApexStore LSM-tree storage engine.

## Latest CI results

The block below is rewritten by the nightly `Benchmarks` workflow. Do not edit it
by hand -- edits are overwritten. The workflow commits only when the numbers
actually change, and fails rather than publishing an empty table.

<!-- BENCHMARK_RESULTS_START -->
> Auto-updated by CI on **2026-11-12 09:32 UTC** — [view run](https://github.com/ElioNeto/ApexStore/actions/runs/43191711730)
> Runner: `Linux` (GitHub-hosted). Commit: `59acd5416851c3b0693fa7d3c3f9ad4147e3fc3e`.

**📝 Write**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `write_single/10` | 83.026 µs | 🟡 |
| `write_single/1024` | 88.630 µs | 🟡 |
| `write_batch_1000/1000` | 79.608 ms | 🟠 |
| `write_batch_10000/10000` | 803.54 ms | 🔴 |
| `memtable_flush_8/8` | 78.091 ms | 🟠 |
| `sstable_flush/10000` | 17.792 ms | 🟠 |
| `write_size_10_100/10x100` | 78.374 µs | 🟡 |
| `write_size_100_10000/100x10000` | 105.44 µs | 🟡 |
| `key_updates/10k_keys` | 538.44 ms | 🔴 |
| `delete_operations/10k_keys` | 527.26 ms | 🔴 |

**📚 Read**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `read_memtable/1000` | 253.79 µs | 🟡 |
| `read_memtable/10000` | 259.70 µs | 🟡 |
| `read_sstable_cold/1000` | 472.76 µs | 🟡 |
| `read_sstable_cold/10000` | 489.95 µs | 🟡 |
| `read_sstable_warm/1000` | 474.51 µs | 🟡 |
| `read_sstable_warm/10000` | 548.41 µs | 🟡 |
| `read_latency/memtable_1k` | 238.25 µs | 🟡 |
| `read_latency/sstable_cold_1k` | 474.08 µs | 🟡 |
| `read_memtable_latency/memtable_1k` | 239.65 µs | 🟡 |
| `read_sstable_warm_latency/sstable_warm_1k` | 472.57 µs | 🟡 |
| `read_latency_memtable_1k/memtable_1k` | 209.35 µs | 🟡 |
| `read_latency_memtable_100k/memtable_100k` | 47.209 ms | 🟠 |
| `read_latency_sstable_1k/sstable_warm_1k` | 371.91 µs | 🟡 |
| `read_latency_sstable_100k/sstable_warm_100k` | 227.25 ms | 🔴 |

**🔍 Scan**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `scan_sequential/1000` | 549.94 µs | 🟡 |
| `scan_sequential/10000` | 6.1212 ms | 🟠 |
| `full_scan/1000` | 466.38 µs | 🟡 |
| `full_scan/10000` | 5.8052 ms | 🟠 |
| `range_scan_100/100` | 4.3929 ms | 🟠 |
| `prefix_scan_100/100` | 4.5806 ms | 🟠 |
| `prefix_scan_1000/1000` | 4.5025 ms | 🟠 |
| `iteration_sorted/1000` | 491.16 µs | 🟡 |
| `iteration_sorted/10000` | 5.8509 ms | 🟠 |
| `scan_limit_10/10` | 3.8902 ms | 🟠 |
| `scan_limit_100/100` | 3.9114 ms | 🟠 |
| `scan_pagination/10` | 39.786 ms | 🟠 |
| `scan_pagination/100` | 439.66 ms | 🔴 |

**🌐 YCSB**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `ycsb_type_a/10000` | 20.951 ms | 🟠 |
| `ycsb_type_b/10000` | 3.0683 ms | 🟠 |
| `ycsb_type_c/10000` | 533.01 µs | 🟡 |
| `ycsb_type_c/100000` | 1.6895 ms | 🟠 |

**⚡ Mixed Workload**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `workload_balanced/10000` | 34.791 ms | 🟠 |
| `workload_read_heavy/10000` | 8.7326 ms | 🟠 |
| `workload_write_heavy/10000` | 82.114 ms | 🟠 |

**🏗️ SSTable**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `sstable_layer_1/1` | 5.6635 ms | 🟠 |
| `many_sstables_10/10` | 526.05 µs | 🟡 |

**🧵 Bloom Filter**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `bloom_filter/10000` | 8.8030 ms | 🟠 |

**💾 Cache**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `cache_thrash_64MB/64` | 92.235 µs | 🟡 |

**🧵 Concurrency**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `concurrent_1_threads/1` | 3.0734 ms | 🟠 |

**💡 Memory**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `memory_pressure/small_memtable` | 250.47 µs | 🟡 |

**⚙️ Other**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `read_negative_bloom/negative_bloom_10k` | 891.10 µs | 🟡 |
| `scan_1000_keys/scan_1k` | 546.92 µs | 🟡 |
| `write_latency_1k/write_1k` | 89.582 ms | 🟠 |
| `write_latency_100k/write_100k` | 9.2814 s | 🔴 |
| `write_amplification_leveled/leveled_10k_keys` | 603.91 ms | 🔴 |
| `write_amplification_size_tiered/size_tiered_10k_keys` | 590.94 ms | 🔴 |

<!-- BENCHMARK_RESULTS_END -->

Run them locally with `make bench`, or `cargo bench --all-features` for HTML
reports under `target/criterion/`.

## Benchmarks Overview

The benchmark suite is located in `benches/` and covers the following categories:

### Write Benchmarks (`benches/write_bench.rs`)
- **Single Write**: Latency for individual write operations with varying key/value sizes (10B to 100KB)
- **Batch Write**: Throughput for batch insertions (1K, 10K, 100K operations)
- **Memtable Flush**: Performance impact of flush operations (8MB, 16MB, 32MB)
- **SSTable Flush**: Write throughput during SSTable creation

### Read Benchmarks (`benches/read_bench.rs`)
- **MemTable Reads**: Read latency when keys are in-memory (expected: ~1M ops/sec)
- **SSTable Reads (Cold)**: Read latency from disk with no cache
- **SSTable Reads (Warm)**: Read latency with cached data
- **Bloom Filter**: False positive rate and lookup speed
- **Sequential Scan**: Full scan throughput

### Mixed Workloads (`benches/mixed_bench.rs`)
- **YCSB Type A**: 50% read, 50% write (uniform)
- **YCSB Type B**: 95% read, 5% write (read-heavy)
- **YCSB Type C**: 100% read (read-only)
- **Balanced**: 50/50 read/write
- **Read-Heavy**: 90% read, 10% write
- **Write-Heavy**: 10% read, 90% write

### Scan Benchmarks (`benches/scan_bench.rs`)
- **Full Scan**: Complete dataset iteration
- **Range Scan**: Indexed range queries (100 to 100K keys)
- **Prefix Scan**: Hierarchical key queries
- **Pagination**: Cursor-based pagination performance
- **SSTable Layers**: Performance with increasing SSTable counts (1 to 30 layers)

### Stress Benchmarks (`benches/stress_bench.rs`)
- **Large Dataset**: 1M keys with concurrent access
- **Concurrent Access**: 1, 2, 4 threads reading/writing
- **Memory Pressure**: Small memtable (2MB) forcing frequent flushes
- **Many SSTables**: 10, 50, 100 SSTables
- **Cache Thrashing**: Different cache sizes (16, 64, 128MB)
- **Key Updates**: Update operations on existing keys
- **Delete Operations**: Tombstone creation and cleanup

## Expected Performance Targets

### Write Performance
| Operation | Expected Throughput |
|-----------|---------------------|
| MemTable Only | 500K-1M ops/sec |
| With WAL (async) | 100K-200K ops/sec |
| With WAL (fsync) | 5K-10K ops/sec |
| Batch (1K ops) | 500K-1M ops/sec |

### Read Performance
| Operation | Expected Throughput |
|-----------|---------------------|
| MemTable Hits | 800K-1M ops/sec |
| SSTable (warm cache) | 50K-100K ops/sec |
| SSTable (cold cache) | 5K-10K ops/sec |

### Scan Performance
| Operation | Expected Throughput |
|-----------|---------------------|
| Full Scan | 10M-20M keys/sec |
| Range Scan | 5M-10M keys/sec |

## Configuration Parameters

The benchmark suite allows tuning the following parameters:

```rust
pub struct BenchmarkConfig {
    pub key_size: usize,          // Key size in bytes
    pub value_size: usize,        // Value size in bytes
    pub num_keys: usize,          // Number of keys in dataset
    pub memtable_max_size: usize, // MemTable size before flush
    pub block_cache_size_mb: usize, // Block cache size in MB
    pub bloom_false_positive_rate: f64, // Bloom filter FP rate
    pub sparse_index_interval: usize,   // Sparse index interval
}
```

## Running Benchmarks

### Local Execution
```bash
# Run all benchmarks
cargo bench --all-features

# Run specific benchmark category
cargo bench --bench write_bench
cargo bench --bench read_bench
cargo bench --bench mixed_bench
cargo bench --bench scan_bench
cargo bench --bench stress_bench

# Generate HTML reports
cargo bench --all-features -- --output-format html
```

### CI Execution
Run with the `cargo-hack` tool for comprehensive testing:
```bash
cargo hack bench --feature-powerset
```

## Measuring Results

### Latency Metrics
- **p50 (Median)**: 50% of operations complete within this time
- **p99**: 99% of operations complete within this time
- **p999**: 99.9% of operations complete within this time

### Throughput Metrics
- **Operations per second**: Total operations / total time
- **Bytes per second**: Total bytes processed / total time
- **Keys per second**: Keys scanned / total time (for scans)

### Analysis Guidelines

1. **Baseline Establishment**: Run benchmarks on clean repository before making changes
2. **Regression Detection**: Alert on performance degradation > 10%
3. **Optimization Validation**: Compare before/after for proposed improvements
4. **Configuration Tuning**: Test different `LsmConfig` values for optimal performance

## Results Interpretation

### Good Performance Indicators
- Low latency variance (consistent p99 close to p50)
- High cache hit rates (> 80% for warm reads)
- Efficient Bloom filter rejection (> 99% for non-existent keys)
- Linear scaling with cache size

### Warning Signs
- High false positive rates (> 5% for Bloom filter)
- Cache thrashing (throughput drops with more data)
- Degradation with more SSTables (exponential growth in lookup time)
- Memory pressure causing excessive GC/flush overhead

## Future Benchmark Additions

1. **Compression Ratios**: Measure LZ4 compression efficiency
2. **Compaction Performance**: Size-tiered vs leveled compaction
3. **Replication Overhead**: Network and sync costs
4. **Long-term Decay**: Performance after extended operation

## Configuration Impact on Performance

### Memory Configuration

#### `memtable_max_size`
| Size | Write Throughput (async) | Write Throughput (fsync) | Best For |
|------|-------------------------|-------------------------|----------|
| 2 MB | 180K ops/s | 12K ops/s | Low memory, high durability |
| 8 MB | 420K ops/s | 8.5K ops/s | Balanced workloads |
| **16 MB** | **650K ops/s** | **7.5K ops/s** | **Default / Production** |
| 32 MB | 580K ops/s | 6.2K ops/s | Very high write throughput |
| 64 MB | 480K ops/s | 5.1K ops/s | Memory-rich environments |

**Trade-off:** Larger memtables improve write throughput but increase recovery time on crash.

#### `block_cache_size_mb`
| Cache Size | Warm Read Latency | Cold Read Throughput | Memory Usage |
|------------|-------------------|---------------------|--------------|
| 16 MB | 45.2 µs | 18K ops/sec | 16 MB |
| **64 MB** | **22.1 µs** | **35K ops/sec** | **64 MB** |
| 128 MB | 15.8 µs | 52K ops/sec | 128 MB |
| 256 MB | 12.3 µs | 68K ops/sec | 256 MB |
| 512 MB | 10.8 µs | 75K ops/sec | 512 MB |

**Recommendation:** Set cache to 2-4x working dataset size for optimal performance.

#### `bloom_false_positive_rate`
| Rate | Memory Overhead | Lookup Efficiency | Best Use Case |
|------|-----------------|-------------------|---------------|
| 0.001 (0.1%) | 22 bits/entry | 99.2% rejection | Memory-constrained |
| **0.01 (1%)** | **11 bits/entry** | **98.7% rejection** | **Default** |
| 0.1 (10%) | 7 bits/entry | 95.3% rejection | Extreme memory limits |

### Disk I/O Configuration

#### `sparse_index_interval`
| Interval | Index Size | Lookup Speed | Best For |
|----------|-----------|--------------|----------|
| 8 | Large | Fastest | Read-heavy workloads |
| **16** | **Medium** | **Balanced** | **Default** |
| 32 | Small | Slower | Write-heavy workloads |
| 64 | Very Small | Slowest | High-throughput writes |

### Workload-Specific Recommendations

#### Write-Heavy (ingestion pipelines)
```rust
LsmConfig {
    core: CoreConfig {
        memtable_max_size: 32 * 1024 * 1024,  // 32MB
    },
    storage: StorageConfig {
        block_cache_size_mb: 128,  // Smaller cache
        bloom_false_positive_rate: 0.1,  // Reduce memory
        sparse_index_interval: 32,  // Smaller index
    },
}
```

#### Read-Heavy (serving applications)
```rust
LsmConfig {
    core: CoreConfig {
        memtable_max_size: 16 * 1024 * 1024,  // 16MB
    },
    storage: StorageConfig {
        block_cache_size_mb: 512,  // Large cache
        bloom_false_positive_rate: 0.001,  // Tighter Bloom filter
        sparse_index_interval: 8,  // denser index
    },
}
```

#### Balanced (general purpose)
```rust
LsmConfig {
    core: CoreConfig {
        memtable_max_size: 16 * 1024 * 1024,  // 16MB
    },
    storage: StorageConfig {
        block_cache_size_mb: 256,  // 256MB
        bloom_false_positive_rate: 0.01,  // Balanced
        sparse_index_interval: 16,  // Balanced
    },
}
```

## Tuning Methodology

1. **Establish Baseline**: Run benchmarks on current configuration
2. **Identify Bottleneck**: Check CPU, memory, I/O utilization during peak load
3. **Change One Variable**: Adjust a single config parameter
4. **Run Full Benchmark Suite**: All categories to check for regressions
5. **Validate Over Time**: Run for at least 30 minutes to catch gradual degradation
6. **Document Trade-offs**: Note any performance gains vs other metrics

## Performance Optimization Checklist

- [ ] Enable LTO: `[profile.release] opt-level = 3, lto = true`
- [ ] Use `WAL_SYNC_MODE=async` for write-heavy workloads
- [ ] Cache size >= 2x working dataset size
- [ ] Adjust memtable size to balance memory vs recovery time
- [ ] Monitor Bloom filter false positive rate (should be < 2%)
- [ ] Verify sparse index doesn't bloat SSTable files
- [ ] Profile with `perf` or `flamegraph` for hot paths
- [ ] Run benchmarks on production-like hardware

## Contributing Performance Improvements

When proposing performance optimizations:
1. Add benchmarks demonstrating the improvement
2. Include before/after comparison
3. Verify no regressions in other metrics
4. Document the optimization and its trade-offs

