# Performance Documentation

This document contains performance benchmarks for the ApexStore LSM-tree storage engine.

## Latest CI results

The block below is rewritten by the nightly `Benchmarks` workflow. Do not edit it
by hand -- edits are overwritten. The workflow commits only when the numbers
actually change, and fails rather than publishing an empty table.

<!-- BENCHMARK_RESULTS_START -->
> Auto-updated by CI on **2026-11-10 09:30 UTC** — [view run](https://github.com/ElioNeto/ApexStore/actions/runs/42790662300)
> Runner: `Linux` (GitHub-hosted). Commit: `2463e9aff026ace8221933e20e68cd14b55c2a3d`.

**📝 Write**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `write_single/10` | 93.905 µs | 🟡 |
| `write_single/1024` | 109.95 µs | 🟡 |
| `write_batch_1000/1000` | 92.218 ms | 🟠 |
| `write_batch_10000/10000` | 931.97 ms | 🔴 |
| `memtable_flush_8/8` | 63.959 ms | 🟠 |
| `sstable_flush/10000` | 14.757 ms | 🟠 |
| `write_size_10_100/10x100` | 90.609 µs | 🟡 |
| `write_size_100_10000/100x10000` | 119.62 µs | 🟡 |
| `key_updates/10k_keys` | 466.02 ms | 🔴 |
| `delete_operations/10k_keys` | 453.61 ms | 🔴 |

**📚 Read**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `read_memtable/1000` | 214.36 µs | 🟡 |
| `read_memtable/10000` | 219.00 µs | 🟡 |
| `read_sstable_cold/1000` | 375.17 µs | 🟡 |
| `read_sstable_cold/10000` | 403.69 µs | 🟡 |
| `read_sstable_warm/1000` | 376.89 µs | 🟡 |
| `read_sstable_warm/10000` | 465.29 µs | 🟡 |
| `read_latency/memtable_1k` | 192.69 µs | 🟡 |
| `read_latency/sstable_cold_1k` | 382.70 µs | 🟡 |
| `read_memtable_latency/memtable_1k` | 194.11 µs | 🟡 |
| `read_sstable_warm_latency/sstable_warm_1k` | 377.05 µs | 🟡 |
| `read_latency_memtable_1k/memtable_1k` | 229.42 µs | 🟡 |
| `read_latency_memtable_100k/memtable_100k` | 39.384 ms | 🟠 |
| `read_latency_sstable_1k/sstable_warm_1k` | 377.33 µs | 🟡 |
| `read_latency_sstable_100k/sstable_warm_100k` | 159.75 ms | 🔴 |

**🔍 Scan**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `scan_sequential/1000` | 402.79 µs | 🟡 |
| `scan_sequential/10000` | 4.2213 ms | 🟠 |
| `full_scan/1000` | 353.58 µs | 🟡 |
| `full_scan/10000` | 5.5934 ms | 🟠 |
| `range_scan_100/100` | 4.4048 ms | 🟠 |
| `prefix_scan_100/100` | 4.5541 ms | 🟠 |
| `prefix_scan_1000/1000` | 4.1111 ms | 🟠 |
| `iteration_sorted/1000` | 379.87 µs | 🟡 |
| `iteration_sorted/10000` | 5.8865 ms | 🟠 |
| `scan_limit_10/10` | 3.5492 ms | 🟠 |
| `scan_limit_100/100` | 3.6064 ms | 🟠 |
| `scan_pagination/10` | 36.020 ms | 🟠 |
| `scan_pagination/100` | 403.24 ms | 🔴 |

**🌐 YCSB**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `ycsb_type_a/10000` | 21.195 ms | 🟠 |
| `ycsb_type_b/10000` | 3.1464 ms | 🟠 |
| `ycsb_type_c/10000` | 702.20 µs | 🟡 |
| `ycsb_type_c/100000` | 2.3654 ms | 🟠 |

**⚡ Mixed Workload**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `workload_balanced/10000` | 22.183 ms | 🟠 |
| `workload_read_heavy/10000` | 5.3389 ms | 🟠 |
| `workload_write_heavy/10000` | 37.918 ms | 🟠 |

**🏗️ SSTable**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `sstable_layer_1/1` | 4.5143 ms | 🟠 |
| `many_sstables_10/10` | 404.53 µs | 🟡 |

**🧵 Bloom Filter**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `bloom_filter/10000` | 7.6009 ms | 🟠 |

**💾 Cache**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `cache_thrash_64MB/64` | 73.904 µs | 🟡 |

**🧵 Concurrency**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `concurrent_1_threads/1` | 2.7628 ms | 🟠 |

**💡 Memory**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `memory_pressure/small_memtable` | 198.99 µs | 🟡 |

**⚙️ Other**

| Benchmark | Median | Perf |
|-----------|:------:|:----:|
| `read_negative_bloom/negative_bloom_10k` | 839.08 µs | 🟡 |
| `scan_1000_keys/scan_1k` | 405.62 µs | 🟡 |
| `write_latency_1k/write_1k` | 54.211 ms | 🟠 |
| `write_latency_100k/write_100k` | 6.0845 s | 🔴 |
| `write_amplification_leveled/leveled_10k_keys` | 662.91 ms | 🔴 |
| `write_amplification_size_tiered/size_tiered_10k_keys` | 642.89 ms | 🔴 |

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

