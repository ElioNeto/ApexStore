# ApexStore Configuration Guide

This guide explains all configuration parameters available in ApexStore. All settings can be configured via environment variables without recompilation.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Server Configuration](#server-configuration)
3. [LSM Engine Configuration](#lsm-engine-configuration)
4. [Performance Tuning](#performance-tuning)
5. [Tuning Profiles](#tuning-profiles)
6. [Troubleshooting](#troubleshooting)

## Quick Start

```bash
# 1. Copy the example configuration
cp .env.example .env

# 2. Edit values as needed
nano .env

# 3. Run the server
cargo run --release --features api --bin apexstore-server
```

The server will load `.env` automatically and display all active configuration on startup.

## Server Configuration

### Network Settings

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Server bind address (0.0.0.0 = all interfaces) |
| `PORT` | `8080` | Server port |

### Payload Limits

| Variable | Default | Description |
|----------|---------|-------------|
| `MAX_JSON_PAYLOAD_SIZE` | `52428800` (50MB) | Maximum JSON request/response size |
| `MAX_RAW_PAYLOAD_SIZE` | `52428800` (50MB) | Maximum raw payload size |

**Recommendations:**
- **Development/Testing**: 50-100MB
- **Production with pagination**: 10MB
- **Stress testing**: 100-200MB

### HTTP Server Tuning

| Variable | Default | Description |
|----------|---------|-------------|
| `SERVER_WORKERS` | `0` (CPU cores) | Number of worker threads |
| `SERVER_KEEP_ALIVE` | `75` | Keep-alive timeout (seconds) |
| `SERVER_CLIENT_TIMEOUT` | `60` | Client request timeout (seconds) |
| `SERVER_SHUTDOWN_TIMEOUT` | `30` | Graceful shutdown timeout (seconds) |
| `SERVER_BACKLOG` | `2048` | Maximum pending connections |
| `SERVER_MAX_CONNECTIONS` | `25000` | Max concurrent connections per worker |

**Recommendations:**
- **High-traffic**: Increase `SERVER_WORKERS` to 8-16
- **Low-latency**: Set `SERVER_KEEP_ALIVE` to 5-15
- **Memory-constrained**: Reduce `SERVER_MAX_CONNECTIONS` to 5000-10000

### Request Rate Limits

`RATE_LIMIT_REQUESTS_PER_MINUTE` defaults to `100` accepted requests per client
IP in a sliding 60-second window, shared across routes. `0` disables this global
cap while keeping configured endpoint caps active. The server additionally
limits `/admin/compact` and `/admin/flush` to five accepted requests each per
window. Rejected requests do not extend the window or consume another cap.

Applications configuring `RateLimiterState::set_endpoint_limit` must use the
registered route pattern, including its scope (for example `/api/keys/{key}`),
not a concrete key URL. A zero endpoint cap disables only that additional cap.
The admin rate-limit summary reports both per-IP and per-route accepted counts
in the last 60 seconds; requests with both caps disabled are not tracked.

Tracking memory is bounded independently from the request budgets:

| Variable | Default | Scope |
|----------|---------|-------|
| `RATE_LIMIT_MAX_IPS_PER_SHARD` | `1024` | Each of 16 rate-limit shards, shared across workers |
| `CONNECTION_MAX_TRACKED_IPS` | `16384` | Distinct IPs with in-flight requests, shared across workers |

A full rate-limit shard responds with 429 for a new peer until a tracked peer's
entire accepted-request window expires. Existing peers retain their budgets;
new addresses cannot evict a live budget and reset its counter. Expiration uses
an ordered index with one entry per peer, avoiding a full-map scan on each
request. Uneven hashing can fill one shard before the total reaches 16,384.

The connection guard also returns 429 for new peers at capacity. Existing peers
can acquire slots up to their per-IP limit. An IP's entry is removed only when
its last in-flight request completes, is cancelled, or unwinds. Active counts
are never evicted. Raising these capacities increases memory use; setting either
to `0` refuses new tracked peers, rather than disabling protection. Unset or
invalid numeric values use the finite defaults above. Requests with both rate
caps disabled remain untracked by the rate limiter.

`GET /admin/rate_limits` preserves its existing fields and adds
`tracked_ip_count`, `max_ips_per_shard`, `shard_count`, `expired_ips`, and
`capacity_rejections`. Its `connections` object reports `tracked_ip_count`,
`max_tracked_ips`, `active_requests`, `max_per_ip`, and `capacity_rejections`.
Counters are process-local and reset on restart. Capacity rejections count only
new peers refused because tracking is full, not ordinary per-IP limit failures.
Expired IPs are removals after the full window, not evictions of live budgets.
The endpoint still requires Admin permission when authentication is enabled.

Embedded applications can use `RateLimiterState::with_capacity` and
`IpConnectionGuard::with_capacity`. `ServerConfig` now has
`rate_limit_max_ips_per_shard` and `connection_max_tracked_ips` fields; serialized
configurations that omit them receive defaults, while exhaustive Rust struct
literals must supply them or use `..ServerConfig::default()`.

The current rate limiter identifies clients by the TCP peer address, matching
the concurrent connection guard. It ignores `X-Forwarded-For` and `Forwarded`,
including malformed values. Clients cannot reset their budget by changing a
header. Behind a reverse proxy, clients share that proxy's peer-address budget;
apply individual client limits at the proxy as well. Configurable trusted-proxy
CIDRs and right-to-left forwarding-chain resolution are not implemented yet.
Do not rely on the old leftmost `X-Forwarded-For` behavior for attribution.

## LSM Engine Configuration

### Storage Settings

| Variable | Default | Description |
|----------|---------|-------------|
| `DATA_DIR` | `./.lsm_data` | Data storage directory path |

### MemTable

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMTABLE_MAX_SIZE` | `4194304` (4MB) | Size threshold before flush to disk |

**Impact:**
- **Larger** (8-16MB): Fewer flushes, better compression, higher memory usage
- **Smaller** (1-2MB): More flushes, lower memory usage, faster recovery

**Recommendations:**
- **Write-heavy**: 8-16MB
- **Memory-constrained**: 2MB
- **Balanced**: 4MB (default)

### SSTable Block Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `BLOCK_SIZE` | `4096` (4KB) | Block size for SSTables |
| `BLOCK_CACHE_SIZE_MB` | `64` | In-memory cache for blocks (MB) |
| `SPARSE_INDEX_INTERVAL` | `16` | Blocks between index entries |

**Block Size Impact:**
- **Larger** (8KB): Better compression ratio, higher read latency
- **Smaller** (2KB): Lower latency, less compression

**Cache Size Recommendations:**
- **Read-heavy**: 256-512MB
- **Balanced**: 64-128MB
- **Memory-constrained**: 32MB

**Sparse Index:**
- **Dense** (8): More memory, faster lookups
- **Sparse** (32): Less memory, slower lookups

### Bloom Filter

| Variable | Default | Description |
|----------|---------|-------------|
| `BLOOM_FALSE_POSITIVE_RATE` | `0.01` (1%) | False positive probability |

**Impact:**
- **Lower** (0.001 = 0.1%): More accurate, more memory
- **Higher** (0.05 = 5%): Less accurate, less memory

**Recommendations:**
- **Read-heavy**: 0.001-0.005
- **Balanced**: 0.01
- **Memory-constrained**: 0.05

### Write-Ahead Log (WAL)

| Variable | Default | Description |
|----------|---------|-------------|
| `MAX_WAL_RECORD_SIZE` | `33554432` (32MB) | Maximum single record size |
| `WAL_BUFFER_SIZE` | `65536` (64KB) | Write buffer size |
| `WAL_SYNC_MODE` | `always` | Fsync strategy |

**Sync Modes:**
- `always`: Safest, slowest (every write synced)
- `every_second`: Balanced (1s of data loss possible)
- `manual`: Fastest, least safe (crash = data loss)

**Recommendations:**
- **Production**: `always`
- **High-throughput**: `every_second`
- **Testing/Dev**: `manual`

### Compaction

| Variable | Default | Description |
|----------|---------|-------------|
| `COMPACTION_STRATEGY` | `lazy_leveling` | Compaction algorithm |
| `SIZE_RATIO` | `10` | Size ratio between levels |
| `LEVEL0_COMPACTION_THRESHOLD` | `4` | L0 file count trigger |
| `MAX_LEVEL_COUNT` | `7` | Maximum LSM tree levels |
| `COMPACTION_THREADS` | `2` | Background compaction threads |

**Compaction Strategies:**
- `leveled`: Best read performance
- `tiered`: Best write performance
- `lazy_leveling`: Balanced (default)

**Recommendations:**
- **Read-heavy**: `leveled`, SIZE_RATIO=4-6
- **Write-heavy**: `tiered`, SIZE_RATIO=15-20
- **High-throughput**: COMPACTION_THREADS=4-8

### Feature Flags

| Variable | Default | Description |
|----------|---------|-------------|
| `FEATURE_CACHE_TTL` | `10` | Cache TTL in seconds |

## Performance Tuning

### Memory vs. Performance Trade-offs

```
High Memory → High Performance:
MEMTABLE_MAX_SIZE=16777216         # 16MB
BLOCK_CACHE_SIZE_MB=512            # 512MB
BLOOM_FALSE_POSITIVE_RATE=0.001    # 0.1%
SPARSE_INDEX_INTERVAL=8            # Dense

Low Memory → Acceptable Performance:
MEMTABLE_MAX_SIZE=2097152          # 2MB
BLOCK_CACHE_SIZE_MB=32             # 32MB
BLOOM_FALSE_POSITIVE_RATE=0.05     # 5%
SPARSE_INDEX_INTERVAL=32           # Sparse
```

### Latency vs. Throughput

```
Low Latency:
BLOCK_SIZE=2048                    # 2KB
WAL_SYNC_MODE=every_second
SERVER_KEEP_ALIVE=5

High Throughput:
BLOCK_SIZE=8192                    # 8KB
MEMTABLE_MAX_SIZE=16777216         # 16MB
COMPACTION_THREADS=8
WAL_BUFFER_SIZE=262144             # 256KB
```

## Tuning Profiles

### Stress Testing Profile

```bash
MAX_JSON_PAYLOAD_SIZE=104857600    # 100MB
MAX_RAW_PAYLOAD_SIZE=104857600
MEMTABLE_MAX_SIZE=16777216         # 16MB
BLOCK_SIZE=8192
BLOCK_CACHE_SIZE_MB=256
WAL_SYNC_MODE=every_second
COMPACTION_THREADS=4
```

### High Write Throughput

```bash
MEMTABLE_MAX_SIZE=8388608          # 8MB
BLOCK_SIZE=8192
BLOOM_FALSE_POSITIVE_RATE=0.05
WAL_SYNC_MODE=every_second
WAL_BUFFER_SIZE=262144             # 256KB
COMPACTION_THREADS=4
LEVEL0_COMPACTION_THRESHOLD=8
COMPACTION_STRATEGY=tiered
```

### High Read Throughput

```bash
BLOCK_CACHE_SIZE_MB=512
BLOOM_FALSE_POSITIVE_RATE=0.001
SPARSE_INDEX_INTERVAL=8
COMPACTION_STRATEGY=leveled
SIZE_RATIO=4
```

### Memory Constrained

```bash
MEMTABLE_MAX_SIZE=2097152          # 2MB
BLOCK_CACHE_SIZE_MB=32
BLOOM_FALSE_POSITIVE_RATE=0.05
SPARSE_INDEX_INTERVAL=32
SERVER_MAX_CONNECTIONS=5000
COMPACTION_THREADS=1
```

### Balanced Production

```bash
MEMTABLE_MAX_SIZE=4194304          # 4MB
BLOCK_SIZE=4096
BLOCK_CACHE_SIZE_MB=128
BLOOM_FALSE_POSITIVE_RATE=0.01
WAL_SYNC_MODE=always
COMPACTION_THREADS=2
SERVER_WORKERS=4
SERVER_MAX_CONNECTIONS=10000
```

## Troubleshooting

### High Memory Usage

1. Reduce `MEMTABLE_MAX_SIZE`
2. Reduce `BLOCK_CACHE_SIZE_MB`
3. Increase `BLOOM_FALSE_POSITIVE_RATE`
4. Increase `SPARSE_INDEX_INTERVAL`
5. Reduce `SERVER_MAX_CONNECTIONS`

### Slow Writes

1. Increase `MEMTABLE_MAX_SIZE`
2. Change `WAL_SYNC_MODE` to `every_second`
3. Increase `WAL_BUFFER_SIZE`
4. Increase `COMPACTION_THREADS`
5. Use `COMPACTION_STRATEGY=tiered`

### Slow Reads

1. Increase `BLOCK_CACHE_SIZE_MB`
2. Decrease `BLOOM_FALSE_POSITIVE_RATE`
3. Decrease `SPARSE_INDEX_INTERVAL`
4. Use `COMPACTION_STRATEGY=leveled`
5. Decrease `SIZE_RATIO`

### Payload Too Large Errors

1. Increase `MAX_JSON_PAYLOAD_SIZE`
2. Increase `MAX_RAW_PAYLOAD_SIZE`
3. Consider implementing pagination

### Too Many Open Files

1. Increase system file descriptor limit: `ulimit -n 65536`
2. Reduce `MAX_LEVEL_COUNT`
3. Decrease `LEVEL0_COMPACTION_THRESHOLD`
4. Increase `SIZE_RATIO`

## Monitoring Configuration Impact

```bash
# Enable detailed logging
RUST_LOG=debug cargo run --features api --bin apexstore-server

# Watch server startup for configuration values
# The server prints all active config on startup

# Monitor metrics via /stats endpoint
curl http://localhost:8080/stats/all
```

## Best Practices

1. **Start with defaults** - They work well for most workloads
2. **Profile first** - Understand your workload before tuning
3. **Change one at a time** - Easier to understand impact
4. **Monitor metrics** - Use `/stats/all` endpoint
5. **Test in dev** - Before applying to production
6. **Document changes** - Track what works and what doesn't
7. **Use profiles** - Quick starting points for common patterns

## Environment-Specific Configs

### Development
```bash
RUST_LOG=debug
MAX_JSON_PAYLOAD_SIZE=104857600
WAL_SYNC_MODE=manual
```

### Staging
```bash
RUST_LOG=info
MAX_JSON_PAYLOAD_SIZE=52428800
WAL_SYNC_MODE=every_second
```

### Production
```bash
RUST_LOG=warn
MAX_JSON_PAYLOAD_SIZE=10485760
WAL_SYNC_MODE=always
SERVER_WORKERS=8
```

## References

- [LSM-Tree Paper](https://www.cs.umb.edu/~poneil/lsmtree.pdf)
- [RocksDB Tuning Guide](https://github.com/facebook/rocksdb/wiki/RocksDB-Tuning-Guide)
- [Actix-Web Configuration](https://actix.rs/docs/server/)


## Encryption at rest

Encryption is opt-in and disabled by default. For the server, configure
`ENCRYPTION_ENABLED=true` and `ENCRYPTION_KEY_PATH` pointing to a readable file
containing a nonzero 32-byte AES key encoded as 64 hexadecimal characters. Keep
this file outside `DATA_DIR`. Missing/invalid keys and invalid boolean values
stop startup; there is no fallback key. The CLI uses `--encrypt-key-file`.

Programmatic configuration uses `LsmConfig.storage.encryption_enabled` and
`encryption_key_path`, validated by `EngineOptions::try_from_config` and
`LsmEngine::new_from_config`. Runtime keys are redacted in Debug and zeroized on
drop; they are not serialized into layout configuration.

Legacy zero-key databases require the explicit offline `migrate-encryption`
command documented in [the migration guide](https://github.com/ElioNeto/ApexStore/blob/main/MIGRATION_GUIDE.md). Do not delete
WALs or SSTables to work around a key/configuration error.
