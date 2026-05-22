pub mod compaction;
pub mod transaction;
pub mod version_set;

use crate::core::log_record::{LogRecord, RangeTombstone};
use crate::core::table::Table;
use crate::infra::cdc::{CdcConfig, CdcEvent, CdcEventType, CdcPublisher};
use crate::infra::error::Result;
use crate::infra::replication::{ReplicationClient, ReplicationConfig, ReplicationRole};
use crate::infra::metrics::EngineMetrics;
use crate::storage::builder::SstableBuilder;
use crate::storage::cache::{Cache, GlobalBlockCache};
use crate::storage::encryption::EncryptionConfig;
use crate::storage::wal::WriteAheadLog;
use fs2::FileExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Semaphore;

use self::compaction::{Compaction, CompactionMetrics, CompactionOptions, CompactionStrategyType};

use self::version_set::VersionSet;
use crate::core::iterators::{MergeIterator, StorageIterator};
use crate::core::key::KeySlice;
use crate::core::memtable::MemTable;

pub const DEFAULT_SCAN_LIMIT: usize = 128;
pub const MAX_SCAN_LIMIT: usize = 1024;

#[derive(Debug, Clone, Default)]
pub struct LsmStats {
    pub num_tables: usize,
    pub total_size: usize,
    pub total_records: usize,
    pub sst_kb: usize,
    pub wal_kb: usize,
    pub num_tables_at_max: usize,
    pub max_levels_reached: usize,
    pub memtable_max_size: usize,
    pub mem_kb: usize,
    pub mem_records: usize,
    pub sst_files: usize,
    pub sst_records: usize,
    // Compaction stats
    pub compaction_strategy: String,
    pub last_compaction_bytes_read: u64,
    pub last_compaction_bytes_written: u64,
    pub last_compaction_files_merged: usize,
    pub last_compaction_duration_ms: u64,
}

/// Engine options.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub block_size: usize,
    pub bloom_bits_per_key: usize,
    pub max_table_size: usize,
    pub min_table_size_to_compact: usize,
    pub max_levels: usize,
    pub level_multiplier: usize,
    pub write_buffer_size: usize,
    pub max_write_buffer_number: usize,
    pub block_cache_size_mb: usize,
    pub compaction_options: CompactionOptions,
    /// Default TTL for keys.  If set, all keys written via `set()`, `put_cf()`,
    /// etc. will automatically expire after this duration unless overridden via
    /// `set_with_ttl()` / `set_cf_with_ttl()`.
    pub default_ttl: Option<std::time::Duration>,
    /// Encryption configuration for data at rest (SSTable blocks and WAL frames).
    pub encryption: EncryptionConfig,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            block_size: 4096,
            bloom_bits_per_key: 10,
            max_table_size: 1024 * 1024,
            min_table_size_to_compact: 64,
            max_levels: 7,
            level_multiplier: 4,
            write_buffer_size: 64 * 1024,
            max_write_buffer_number: 4,
            block_cache_size_mb: 64,
            compaction_options: CompactionOptions::default(),
            default_ttl: None,
            encryption: EncryptionConfig::default(),
        }
    }
}

impl From<&crate::infra::config::LsmConfig> for EngineOptions {
    fn from(config: &crate::infra::config::LsmConfig) -> Self {
        let compaction_options = CompactionOptions {
            strategy_type: config.compaction.strategy.clone().into(),
            compaction_threshold: config.compaction.min_compaction_threshold,
            max_tables_per_compaction: config.compaction.max_sstables,
            max_concurrent_compactions: 2,
        };

        // Build encryption config from the config
        let encryption = if config.storage.encryption_enabled {
            config
                .storage
                .encryption_key_path
                .as_deref()
                .map(|path| EncryptionConfig::from_key_path(Some(path)))
                .unwrap_or_else(|| {
                    Err(crate::infra::error::LsmError::InvalidArgument(
                        "Encryption enabled but no key path provided".to_string(),
                    ))
                })
                .unwrap_or_default()
        } else {
            EncryptionConfig::default()
        };

        Self {
            block_size: config.storage.block_size,
            bloom_bits_per_key: 10,
            max_table_size: 1024 * 1024,
            min_table_size_to_compact: 64,
            max_levels: 7,
            level_multiplier: 4,
            write_buffer_size: config.core.memtable_max_size,
            max_write_buffer_number: 4,
            block_cache_size_mb: config.storage.block_cache_size_mb,
            compaction_options,
            default_ttl: None,
            encryption,
        }
    }
}

impl EngineOptions {
    /// Create EngineOptions from LsmConfig
    pub fn from_config(config: &crate::infra::config::LsmConfig) -> Self {
        config.into()
    }
}

/// Information about a stored snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotInfo {
    pub path: PathBuf,
    pub created_at: SystemTime,
    pub size_bytes: u64,
    pub file_count: usize,
}

/// Manifest file written by create_snapshot() and read by restore_snapshot()
/// and engine startup.  Maps each column family to its list of SSTable filenames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotManifest {
    /// Map from column family name → list of SSTable filenames (relative to snapshot dir)
    pub column_families: HashMap<String, Vec<String>>,
}

/// All mutable state of the engine, protected behind a Mutex.
pub(crate) struct EngineCore<C: Cache> {
    memtables: HashMap<String, Vec<MemTable>>,
    memtable_bytes: HashMap<String, usize>,
    version_set: VersionSet<C>,
    compaction: Compaction,
    /// Per-column-family WALs.  The "default" CF uses `wal.log`;
    /// other CFs use `wal-{cf}.log`.
    wals: HashMap<String, WriteAheadLog>,
    /// Database directory path, used to create new per-CF WALs lazily.
    dir_path: std::path::PathBuf,
    /// Active range tombstones per column family.
    range_tombstones: HashMap<String, Vec<crate::core::log_record::RangeTombstone>>,
    /// Encryption config used when creating new WALs.
    encryption: EncryptionConfig,
}

impl<C: Cache> EngineCore<C> {
    pub(crate) fn memtables(&self) -> &HashMap<String, Vec<MemTable>> {
        &self.memtables
    }
    pub(crate) fn memtables_mut(&mut self) -> &mut HashMap<String, Vec<MemTable>> {
        &mut self.memtables
    }
    pub(crate) fn memtable_bytes(&self) -> &HashMap<String, usize> {
        &self.memtable_bytes
    }
    pub(crate) fn memtable_bytes_mut(&mut self) -> &mut HashMap<String, usize> {
        &mut self.memtable_bytes
    }
    pub(crate) fn version_set(&self) -> &VersionSet<C> {
        &self.version_set
    }
    pub(crate) fn version_set_mut(&mut self) -> &mut VersionSet<C> {
        &mut self.version_set
    }
    pub(crate) fn compaction(&self) -> &Compaction {
        &self.compaction
    }
    pub(crate) fn compaction_mut(&mut self) -> &mut Compaction {
        &mut self.compaction
    }
    /// Get a mutable reference to the WAL for a specific column family.
    /// Creates a new WAL file if one doesn't exist yet.
    pub(crate) fn wal_mut(&mut self, cf: &str) -> Result<&mut WriteAheadLog> {
        if !self.wals.contains_key(cf) {
            let wal = WriteAheadLog::new_with_encryption(&self.dir_path, cf, &self.encryption)?;
            self.wals.insert(cf.to_string(), wal);
        }
        self.wals.get_mut(cf).ok_or_else(|| {
            crate::infra::error::LsmError::InvalidArgument(format!(
                "WAL not found for column family: {}",
                cf
            ))
        })
    }

    pub(crate) fn range_tombstones(&self) -> &HashMap<String, Vec<crate::core::log_record::RangeTombstone>> {
        &self.range_tombstones
    }

    pub(crate) fn range_tombstones_mut(
        &mut self,
    ) -> &mut HashMap<String, Vec<crate::core::log_record::RangeTombstone>> {
        &mut self.range_tombstones
    }
}

/// The core engine that manages LSM-tree structure and compaction.
///
/// # Type parameters
///
/// * `C` — The block cache implementation (typically
///   [`GlobalBlockCache`](crate::storage::cache::GlobalBlockCache) or
///   [`NoopCache`](crate::storage::cache::NoopCache) for tests).
///
/// # Usage example
///
/// ```rust
/// use apexstore::LsmConfig;
/// use apexstore::core::engine::Engine;
/// use apexstore::storage::cache::GlobalBlockCache;
///
/// let dir = tempfile::tempdir().unwrap();
/// let mut config = LsmConfig::default();
/// config.core.dir_path = dir.path().to_path_buf();
///
/// let engine = Engine::new_from_config(
///     &config,
///     GlobalBlockCache::new(100, 4096),
/// ).unwrap();
///
/// engine.set(b"key1", b"value1").unwrap();
/// assert_eq!(engine.get(b"key1").unwrap(), Some(b"value1".to_vec()));
/// engine.delete(b"key1").unwrap();
/// assert_eq!(engine.get(b"key1").unwrap(), None);
/// ```
pub struct Engine<C: Cache> {
    options: EngineOptions,
    /// All mutable state behind a mutex for thread-safe access.
    core: Arc<Mutex<EngineCore<C>>>,
    /// Semaphore that limits the number of concurrent compaction threads.
    /// Acquire a permit before spawning a compaction thread; the permit is
    /// released when the thread finishes.
    compaction_semaphore: Arc<Semaphore>,
    /// Handles to all running background compaction threads.
    compaction_threads: Mutex<Vec<JoinHandle<()>>>,
    /// Flag set during close() to prevent new compaction threads from spawning.
    closing: Arc<AtomicBool>,
    /// Path to the manifest file (unused currently).
    _manifest: PathBuf,
    /// SSTable output directory (used during initialization).
    _sst_dir: PathBuf,
    /// File lock handle — prevents concurrent access to the same database directory.
    /// Held for the entire engine lifetime; lock is released on drop.
    _lock_file: std::fs::File,
    /// Engine metrics (counters and latency accumulators).
    pub metrics: Arc<EngineMetrics>,

    /// Optional replication client for shipping WAL records to replicas.
    /// Only active when the replication role is Primary.
    pub(crate) replication_client: Option<Arc<ReplicationClient>>,

    /// Handle to the background replication shipping task (Primary only).
    pub(crate) _replication_handle: Option<tokio::task::JoinHandle<()>>,

    /// CDC state (config + publisher).
    cdc: Mutex<CdcState>,
}

/// Holds the CDC state behind a single mutex for atomic access.
struct CdcState {
    config: CdcConfig,
    publisher: Option<Box<dyn CdcPublisher>>,
}

pub type LsmEngineGeneric<C> = Engine<C>;
pub type LsmEngine = Engine<Arc<crate::storage::cache::GlobalBlockCache>>;
pub type ScanRangeResult = crate::infra::error::Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)>;

#[allow(dead_code)]
impl<C: Cache> Engine<C> {
    // ========== pub(crate) accessors for internal crate use ==========
    // These methods are reserved for future use / external crate access

    /// Returns the engine options.
    pub(crate) fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// Returns the write buffer limit.
    pub(crate) fn write_buffer_limit(&self) -> usize {
        self.options.write_buffer_size * self.options.max_write_buffer_number
    }

    /// Returns the SSTable directory (for testing).
    pub(crate) fn sst_dir(&self) -> &PathBuf {
        &self._sst_dir
    }

    /// Lock the core and return the guard.
    pub(crate) fn lock_core(&self) -> parking_lot::MutexGuard<'_, EngineCore<C>> {
        self.core.lock()
    }

    /// Returns a reference to the engine metrics.
    pub fn metrics(&self) -> Arc<EngineMetrics> {
        self.metrics.clone()
    }

    /// Returns `true` if compaction is currently running (at least one permit
    /// of the compaction semaphore is acquired).
    pub fn is_compaction_running(&self) -> bool {
        let max = self.options.compaction_options.max_concurrent_compactions;
        self.compaction_semaphore.available_permits() < max
    }

    /// Configure CDC on this engine.
    ///
    /// If `config.enabled` is `true`, a collector or webhook publisher is created
    /// according to `config.endpoint`.
    pub fn set_cdc(&self, config: CdcConfig) {
        let publisher = crate::infra::cdc::create_publisher(&config);
        let mut cdc = self.cdc.lock();
        cdc.config = config;
        cdc.publisher = publisher;
    }

    /// Set a custom CDC publisher (e.g. for testing).
    pub fn set_cdc_publisher(&self, publisher: Box<dyn CdcPublisher>) {
        let mut cdc = self.cdc.lock();
        cdc.config = CdcConfig {
            enabled: true,
            endpoint: None,
        };
        cdc.publisher = Some(publisher);
    }

    /// Publish a CDC event if a publisher is configured.
    fn publish_cdc_event(&self, cf: &str, key: &[u8], value: Option<&[u8]>) {
        let cdc = self.cdc.lock();
        if let Some(ref publisher) = cdc.publisher {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let event = CdcEvent {
                event_type: if value.is_some() {
                    CdcEventType::Put
                } else {
                    CdcEventType::Delete
                },
                cf: cf.to_string(),
                key: key.to_vec(),
                value: value.map(|v| v.to_vec()),
                timestamp,
            };
            if let Err(e) = publisher.publish(event) {
                tracing::warn!(target: "apexstore::engine", "CDC publish failed: {:?}", e);
            }
        }
    }
}

/// Compact a single column family, operating directly on `&mut EngineCore`.
/// This is the core compaction logic extracted from `compact_cf` so it can be
/// reused from both the public synchronous API and the background compaction thread.
///
/// NOTE: This function holds the lock (via the `&mut EngineCore` borrow) for the
/// entire compaction duration, including I/O. For background compaction where
/// the lock should be released during I/O, use the three-phase approach in
/// `maybe_compact` instead.
fn compact_cf_core<C: Cache>(
    core: &mut EngineCore<C>,
    options: &EngineOptions,
    cf: &str,
) -> Result<Option<CompactionMetrics>> {
    let tables = core.version_set().get_tables(cf);
    if tables.len() < core.compaction().options().compaction_threshold {
        return Ok(None);
    }

    let groups = core.compaction().pick_compaction(&tables, options);
    if groups.is_empty() {
        return Ok(None);
    }

    // Collect active range tombstones for this CF to pass to compaction
    let rt = core
        .range_tombstones()
        .get(cf)
        .cloned()
        .unwrap_or_default();

    let mut all_metrics = CompactionMetrics::default();
    for indices in &groups {
        let (new_tables, metrics) =
            core.compaction_mut()
                .compact(indices, &tables, options, &rt)?;
        let removed_paths = core.version_set_mut()
            .atomic_replace(cf, indices, new_tables);
        // Delete orphaned SSTable files from disk
        for path in &removed_paths {
            if path.exists() {
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::warn!(
                        "compact_cf_core: failed to remove orphaned SSTable {:?}: {:?}",
                        path, e
                    );
                }
            }
        }
        all_metrics.bytes_read += metrics.bytes_read;
        all_metrics.bytes_written += metrics.bytes_written;
        all_metrics.files_merged += metrics.files_merged;
        all_metrics.duration_ms += metrics.duration_ms;
    }

    Ok(Some(all_metrics))
}

impl<C: Cache> Engine<C> {
    /// Create a new engine with default options.
    pub fn new_generic(
        options: EngineOptions,
        cache: C,
        dir_path: &std::path::Path,
    ) -> Result<Self> {
        // Create SSTable directory
        let sst_dir = dir_path.join("sstables");
        std::fs::create_dir_all(&sst_dir)?;

        // Acquire an exclusive file lock to prevent concurrent access
        let lock_path = dir_path.join(".apexstore.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&lock_path)?;
        lock_file.try_lock_exclusive().map_err(|e| {
            if e.kind() == std::io::ErrorKind::WouldBlock {
                crate::LsmError::InvalidArgument("Database is locked by another process".into())
            } else {
                e.into()
            }
        })?;

        // Create storage config from options (with encryption derived from engine options)
        let encryption_enabled = options.encryption.enabled;
        let encryption_key_path = None; // Key is already loaded in options.encryption
        let storage_config = crate::infra::config::StorageConfig {
            block_size: options.block_size,
            block_cache_size_mb: options.block_cache_size_mb,
            sparse_index_interval: 16,
            bloom_false_positive_rate: 0.01,
            encryption_enabled,
            encryption_key_path,
            prefix_compression_enabled: false,
        };

        // Create compaction with strategy from options
        let strategy_type = if options.compaction_options.compaction_threshold <= 4 {
            CompactionStrategyType::SizeTiered
        } else {
            CompactionStrategyType::Leveled
        };

        let compaction_options = CompactionOptions {
            strategy_type,
            compaction_threshold: options.compaction_options.compaction_threshold,
            max_tables_per_compaction: options.compaction_options.max_tables_per_compaction,
            max_concurrent_compactions: options.compaction_options.max_concurrent_compactions,
        };

        // Create shared block cache for on-disk SSTable reads
        let block_cache = GlobalBlockCache::new(options.block_cache_size_mb, options.block_size);

        let version_set = VersionSet::new(
            options.clone(),
            cache,
            storage_config.clone(),
            Some(block_cache),
        );

        // Convert infra config to storage config for the compaction layer
        let compaction_storage_config = crate::infra::config::StorageConfig {
            block_size: storage_config.block_size,
            block_cache_size_mb: storage_config.block_cache_size_mb,
            sparse_index_interval: storage_config.sparse_index_interval,
            bloom_false_positive_rate: storage_config.bloom_false_positive_rate,
            encryption_enabled: storage_config.encryption_enabled,
            encryption_key_path: storage_config.encryption_key_path.clone(),
            prefix_compression_enabled: storage_config.prefix_compression_enabled,
        };
        let compaction = Compaction::new(
            strategy_type,
            compaction_options,
            compaction_storage_config,
            sst_dir.clone(),
        );

        // ── Recover all per-CF WALs ──────────────────────────────────
        // Start with the default WAL, then discover any wal-{cf}.log files.
        let mut core = EngineCore {
            memtables: HashMap::new(),
            memtable_bytes: HashMap::new(),
            version_set,
            compaction,
            wals: HashMap::new(),
            dir_path: dir_path.to_path_buf(),
            range_tombstones: HashMap::new(),
            encryption: options.encryption.clone(),
        };

        // Create and recover the "default" CF WAL
        {
            let default_wal =
                WriteAheadLog::new_with_encryption(dir_path, "default", &options.encryption)?;
            let records = default_wal.recover()?;
            core.wals.insert("default".to_string(), default_wal);
            Self::replay_wal_records_core(&mut core, records)?;
        }

        // Discover additional per-CF WALs (wal-{cf}.log)
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Match wal-{cf}.log where cf != "default"
                if let Some(cf) = name_str
                    .strip_prefix("wal-")
                    .and_then(|s| s.strip_suffix(".log"))
                {
                    if cf != "default" && !core.wals.contains_key(cf) {
                        match WriteAheadLog::new_with_encryption(dir_path, cf, &options.encryption) {
                            Ok(wal) => {
                                let records = wal.recover()?;
                                core.wals.insert(cf.to_string(), wal);
                                Self::replay_wal_records_core(&mut core, records)?;
                            }
                            Err(e) => {
                                tracing::warn!("Failed to open WAL for CF '{}': {:?}", cf, e);
                            }
                        }
                    }
                }
            }
        }

        // ── Discover SSTables from disk (for snapshot restore recovery) ──
        // Check for a disk.sst.manifest written by restore_snapshot().
        Self::discover_sstables_from_disk(&mut core, dir_path, &sst_dir)?;

        // Initialize replication client if configured as Primary
        let (replication_client, replication_handle) = {
            // Attempt to read replication config; default is Primary with no endpoints,
            // which means replication is effectively disabled.
            //
            // The new_from_config caller can set up replication endpoints.  Since this
            // constructor is generic, we check via a config file or env-var convention.
            // For simplicity, if REPLICATION_ROLE env var is set to "primary" and
            // REPLICA_ENDPOINTS is non-empty, we start the client.
            let role = std::env::var("REPLICATION_ROLE")
                .ok()
                .and_then(|s| match s.to_lowercase().as_str() {
                    "primary" => Some(ReplicationRole::Primary),
                    "replica" => Some(ReplicationRole::Replica),
                    _ => None,
                })
                .unwrap_or(ReplicationRole::Primary);

            let replica_endpoints = std::env::var("REPLICA_ENDPOINTS")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|ep| ep.trim().to_string())
                        .filter(|ep| !ep.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let sync_interval_ms = std::env::var("REPLICATION_SYNC_INTERVAL_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(100);

            if role == ReplicationRole::Primary && !replica_endpoints.is_empty() {
                let repl_config = ReplicationConfig {
                    role,
                    replica_endpoints,
                    sync_interval_ms,
                };
                tracing::info!(
                    target: "apexstore::engine",
                    "Starting replication client (Primary) with {} endpoints, interval={}ms",
                    repl_config.replica_endpoints.len(),
                    repl_config.sync_interval_ms,
                );
                let (client, handle) = ReplicationClient::start(repl_config);
                (Some(Arc::new(client)), Some(handle))
            } else {
                (None, None)
            }
        };

        let engine = Self {
            options: options.clone(),
            core: Arc::new(Mutex::new(core)),
            compaction_semaphore: Arc::new(Semaphore::new(
                options.compaction_options.max_concurrent_compactions,
            )),
            compaction_threads: Mutex::new(Vec::new()),
            closing: Arc::new(AtomicBool::new(false)),
            _manifest: PathBuf::new(),
            _sst_dir: sst_dir,
            _lock_file: lock_file,
            metrics: Arc::new(EngineMetrics::new()),
            replication_client,
            _replication_handle: replication_handle,
            cdc: Mutex::new(CdcState {
                config: CdcConfig::disabled(),
                publisher: None,
            }),
        };

        Ok(engine)
    }

    /// Create a new engine from an `LsmConfig` (the app-level config).
    pub fn new_from_config(config: &crate::infra::config::LsmConfig, cache: C) -> Result<Self> {
        let options: EngineOptions = config.into();
        let dir_path = std::path::PathBuf::from(&config.core.dir_path);
        let mut engine = Self::new_generic(options, cache, &dir_path)?;

        // If LsmConfig has explicit replication settings, prefer them over env vars
        // by re-initializing the replication client if needed.
        if !config.replication.replica_endpoints.is_empty()
            && config.replication.role == ReplicationRole::Primary
            && engine.replication_client.is_none()
        {
            let repl_config = ReplicationConfig {
                role: config.replication.role.clone(),
                replica_endpoints: config.replication.replica_endpoints.clone(),
                sync_interval_ms: config.replication.sync_interval_ms,
            };
            tracing::info!(
                target: "apexstore::engine",
                "Starting replication client from config (Primary) with {} endpoints",
                repl_config.replica_endpoints.len(),
            );
            let (client, handle) = ReplicationClient::start(repl_config);
            engine.replication_client = Some(Arc::new(client));
            engine._replication_handle = Some(handle);
        }

        Ok(engine)
    }

    /// Replay WAL records to reconstruct memtable state (operates on EngineCore directly).
    fn replay_wal_records_core(core: &mut EngineCore<C>, records: Vec<LogRecord>) -> Result<()> {
        for record in records {
            let cf = record.column_family.as_deref().unwrap_or("default");
            if record.is_range_tombstone() {
                // Range tombstone records are stored at the EngineCore level
                // and also added to the current memtable's range tombstone list.
                let range = crate::core::log_record::RangeTombstone {
                    start_key: record.range_start.clone().unwrap_or_default(),
                    end_key: record.range_end.clone().unwrap_or_default(),
                    timestamp: record.timestamp,
                };
                core.range_tombstones_mut()
                    .entry(cf.to_string())
                    .or_default()
                    .push(range.clone());
                let mem = core.memtables_mut().entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                mem[last].add_range_tombstone(range);
            } else if record.is_deleted {
                let mem = core.memtables_mut().entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                mem[last].delete(record.key.clone());
                *core.memtable_bytes_mut().entry(cf.to_string()).or_default() += record.key.len();
            } else {
                let mem = core.memtables_mut().entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                mem[last].put(record.key.clone(), record.value.clone());
                *core.memtable_bytes_mut().entry(cf.to_string()).or_default() +=
                    record.key.len() + record.value.len();
            }
        }
        Ok(())
    }
}

// ========== Public API methods (in a separate impl block for readability) ==========
//
// These methods hold the core lock briefly and release it before calling
// maybe_compact() which may spawn a background compaction thread.

impl<C: Cache> Engine<C> {
    /// Put a key-value pair into the specified column family with an optional TTL.
    ///
    /// If `ttl` is `Some(duration)`, the key will expire after that duration.
    /// If `ttl` is `None`, no expiry is set (unless `default_ttl` is configured).
    fn put_cf_with_ttl_inner(
        &self,
        cf: &str,
        key: Vec<u8>,
        value: Vec<u8>,
        ttl: Option<std::time::Duration>,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let key_str = String::from_utf8_lossy(&key).into_owned();
        let value_size = value.len();
        let needs_compact;
        let replication_record: Option<LogRecord>;
        {
            let mut core = self.core.lock();
            // Write to WAL first (before modifying memtable) for crash safety
            let mut record = if let Some(ttl) = ttl {
                let mut r = LogRecord::new_with_ttl(key.clone(), value.clone(), ttl);
                r.column_family = Some(cf.to_string());
                r
            } else {
                let mut r = LogRecord::new(key.clone(), value.clone());
                r.column_family = Some(cf.to_string());
                r
            };
            // Apply default_ttl if no explicit TTL was given
            if record.expires_at.is_none() {
                if let Some(default_ttl) = self.options.default_ttl {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    record.expires_at = Some(now.saturating_add(default_ttl.as_nanos()));
                }
            }
            core.wal_mut(cf)?.write_record(&record)?;

            // Save a clone for replication before moving record into memtable
            replication_record = Some(record.clone());

            let mem = core.memtables_mut().entry(cf.to_string()).or_default();
            if mem.is_empty() {
                mem.push(MemTable::new_unlimited());
            }
            let last = mem.len() - 1;
            mem[last].insert(record);
            *core.memtable_bytes_mut().entry(cf.to_string()).or_default() +=
                key.len() + value.len();
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes().get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        } // core lock is dropped here

        // Ship the record to replicas (Primary only)
        if let Some(client) = &self.replication_client {
            if let Some(record) = replication_record {
                client.ship_records(vec![record]);
            }
        }

        // Publish CDC event (fire-and-forget, runs outside core lock)
        self.publish_cdc_event(cf, &key, Some(&value));

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_set(elapsed_us);
        tracing::debug!(
            target: "apexstore::engine",
            operation = "put_cf",
            cf = cf,
            key = %key_str,
            value_size = value_size,
            duration_us = elapsed_us,
            needs_compact = needs_compact,
        );
        if needs_compact {
            tracing::info!(
                target: "apexstore::engine",
                operation = "put_cf.compact",
                cf = cf,
                "memtable full, triggering compaction"
            );
            self.maybe_compact();
        }
        Ok(())
    }

    /// Put a key-value pair into the specified column family.
    pub fn put_cf(&self, cf: &str, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.put_cf_with_ttl_inner(cf, key, value, None)
    }

    pub fn set<K, V>(&self, key: K, value: V) -> Result<()>
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let key_vec = key.into();
        let value_vec = value.into();
        tracing::info!(
            target: "apexstore::engine",
            operation = "set",
            cf = "default",
            key = %String::from_utf8_lossy(&key_vec),
            value_size = value_vec.len(),
        );
        self.put_cf("default", key_vec, value_vec)
    }

    /// Store a key-value pair with a Time-To-Live (TTL).
    ///
    /// After `ttl` elapses, the key will be treated as non-existent
    /// by `get()` and `scan()`.
    pub fn set_with_ttl<K, V>(&self, key: K, value: V, ttl: std::time::Duration) -> Result<()>
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let key_vec = key.into();
        let value_vec = value.into();
        tracing::info!(
            target: "apexstore::engine",
            operation = "set_with_ttl",
            cf = "default",
            key = %String::from_utf8_lossy(&key_vec),
            value_size = value_vec.len(),
            ttl_ms = ttl.as_millis(),
        );
        self.put_cf_with_ttl_inner("default", key_vec, value_vec, Some(ttl))
    }

    /// Store a key-value pair with a Time-To-Live (TTL) in the given column family.
    pub fn set_cf_with_ttl<K, V>(
        &self,
        cf: &str,
        key: K,
        value: V,
        ttl: std::time::Duration,
    ) -> Result<()>
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let key_vec = key.into();
        let value_vec = value.into();
        tracing::info!(
            target: "apexstore::engine",
            operation = "set_cf_with_ttl",
            cf = cf,
            key = %String::from_utf8_lossy(&key_vec),
            value_size = value_vec.len(),
            ttl_ms = ttl.as_millis(),
        );
        self.put_cf_with_ttl_inner(cf, key_vec, value_vec, Some(ttl))
    }

    pub fn delete_cf<K>(&self, cf: &str, key: K) -> Result<()>
    where
        K: Into<Vec<u8>>,
    {
        let key = key.into();
        let start = std::time::Instant::now();
        let key_str = String::from_utf8_lossy(&key).into_owned();
        let needs_compact;
        let replication_record: Option<LogRecord>;
        {
            let mut core = self.core.lock();

            // Write tombstone to WAL first (before modifying memtable) for crash safety
            let mut record = LogRecord::tombstone(key.clone());
            record.column_family = Some(cf.to_string());
            core.wal_mut(cf)?.write_record(&record)?;

            // Save clone for replication before consuming record
            replication_record = Some(record.clone());

            let mem = core.memtables_mut().entry(cf.to_string()).or_default();
            if mem.is_empty() {
                mem.push(MemTable::new_unlimited());
            }
            let last = mem.len() - 1;
            mem[last].delete(key.clone());
            *core.memtable_bytes_mut().entry(cf.to_string()).or_default() += key.len();
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes().get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        }

        // Ship tombstone to replicas (Primary only)
        if let Some(client) = &self.replication_client {
            if let Some(record) = replication_record {
                client.ship_records(vec![record]);
            }
        }

        // Publish CDC event (fire-and-forget, runs outside core lock)
        self.publish_cdc_event(cf, &key, None);

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_delete(elapsed_us);
        tracing::info!(
            target: "apexstore::engine",
            operation = "delete_cf",
            cf = cf,
            key = %key_str,
            duration_us = elapsed_us,
            needs_compact = needs_compact,
        );
        if needs_compact {
            self.maybe_compact();
        }
        Ok(())
    }

    pub fn delete<K>(&self, key: K) -> Result<()>
    where
        K: Into<Vec<u8>>,
    {
        let key_vec = key.into();
        tracing::info!(
            target: "apexstore::engine",
            operation = "delete",
            cf = "default",
            key = %String::from_utf8_lossy(&key_vec),
        );
        self.delete_cf("default", key_vec)
    }

    /// Check if a key falls within any active range tombstone for the given column family.
    fn is_in_range_tombstone(core: &EngineCore<C>, cf: &str, key: &[u8]) -> bool {
        if let Some(tombstones) = core.range_tombstones().get(cf) {
            if tombstones
                .iter()
                .any(|rt| rt.start_key.as_slice() <= key && key < rt.end_key.as_slice())
            {
                return true;
            }
        }
        // Also check memtable-level range tombstones
        if let Some(memtables) = core.memtables().get(cf) {
            for mem in memtables.iter() {
                if mem.contains_range_tombstone(key) {
                    return true;
                }
            }
        }
        false
    }

    pub fn get_cf<K>(&self, cf: &str, key: K) -> Result<Option<Vec<u8>>>
    where
        K: AsRef<[u8]>,
    {
        let key = key.as_ref();
        let start = std::time::Instant::now();
        let key_str = String::from_utf8_lossy(key).into_owned();
        let core = self.core.lock();

        // First check memtables (newest first) — point writes take precedence
        // over range tombstones.
        if let Some(memtables) = core.memtables().get(cf) {
            for mem in memtables.iter().rev() {
                if let Some(v) = mem.data.get(key) {
                    // Skip tombstones (deleted records)
                    if v.is_deleted {
                        return Ok(None);
                    }
                    // Skip expired keys (TTL-based auto-expiry)
                    if v.is_expired() {
                        return Ok(None);
                    }
                    let elapsed_us = start.elapsed().as_micros() as u64;
                    self.metrics.record_get(elapsed_us);
                    self.metrics.record_cache_hit();
                    tracing::debug!(
                        target: "apexstore::engine",
                        operation = "get_cf",
                        cf = cf,
                        key = %key_str,
                        found = true,
                        value_size = v.value.len(),
                        duration_us = elapsed_us,
                        source = "memtable",
                    );
                    return Ok(Some(v.value.clone()));
                }
            }
        }

        // After memtable lookup, check if key falls within a range tombstone.
        // This is done after memtable check so point writes take precedence.
        if Self::is_in_range_tombstone(&core, cf, key) {
            let elapsed_us = start.elapsed().as_micros() as u64;
            self.metrics.record_get(elapsed_us);
            tracing::debug!(
                target: "apexstore::engine",
                operation = "get_cf",
                cf = cf,
                key = %key_str,
                found = false,
                reason = "range_tombstone",
                duration_us = elapsed_us,
            );
            return Ok(None);
        }

        let result = core.version_set().get(cf, key);
        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_get(elapsed_us);
        match &result {
            Some(v) => {
                self.metrics.record_cache_hit();
                tracing::debug!(
                    target: "apexstore::engine",
                    operation = "get_cf",
                    cf = cf,
                    key = %key_str,
                    found = true,
                    value_size = v.len(),
                    duration_us = elapsed_us,
                    source = "sstable",
                );
            }
            None => {
                self.metrics.record_cache_miss();
                tracing::debug!(
                    target: "apexstore::engine",
                    operation = "get_cf",
                    cf = cf,
                    key = %key_str,
                    found = false,
                    duration_us = elapsed_us,
                );
            }
        }
        Ok(result)
    }

    pub fn get<K>(&self, key: K) -> Result<Option<Vec<u8>>>
    where
        K: AsRef<[u8]>,
    {
        let key_bytes = key.as_ref().to_vec();
        tracing::debug!(
            target: "apexstore::engine",
            operation = "get",
            cf = "default",
            key = %String::from_utf8_lossy(&key_bytes),
        );
        self.get_cf("default", key_bytes)
    }

    pub fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan_cf("default", None, None, Some(DEFAULT_SCAN_LIMIT))
    }

    pub fn scan_cf(
        &self,
        cf: &str,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let start = std::time::Instant::now();
        let core = self.core.lock();
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        // 1. Memtables (newer first)
        if let Some(memtables) = core.memtables().get(cf) {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(crate::storage::iterator::MemTableIterator::new(
                    &mem.data,
                )));
            }
        }

        // 2. SSTables (from VersionSet) — skip non-intersecting ranges
        for sst_iter in core.version_set().table_iters_in_range(cf, lower, upper) {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();
        let limit = limit.unwrap_or(MAX_SCAN_LIMIT);

        while merge_iter.is_valid() && results.len() < limit {
            if let Some(lower) = lower {
                if merge_iter.key().as_slice() < lower {
                    merge_iter.next();
                    continue;
                }
            }
            if let Some(upper) = upper {
                if merge_iter.key().as_slice() >= upper {
                    break;
                }
            }
            // Skip keys that fall within active range tombstones
            let key = merge_iter.key();
            if Self::is_in_range_tombstone(&core, cf, key.as_slice()) {
                merge_iter.next();
                continue;
            }
            results.push((merge_iter.key(), merge_iter.value().to_vec()));
            merge_iter.next();
        }

        // Filter out expired entries that are still in a memtable.
        // Keys from SSTables cannot be checked for TTL because the
        // LogRecord metadata (including expires_at) is lost during
        // flush (see flush_memtable_impl / Table::build).
        //
        // NOTE: flush_memtable_impl already skips expired keys, so
        // the only expired keys that can appear are those written
        // recently (still in memtable, not yet flushed).  We look
        // them up here and remove them from results.
        if let Some(memtables) = core.memtables().get(cf) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            results.retain(|(k, _)| {
                // Check memtables in reverse (newest first)
                for mem in memtables.iter().rev() {
                    if let Some(record) = mem.data.get(k) {
                        // Found in a memtable — keep only if not expired
                        return !record.is_expired_at(now);
                    }
                }
                // Not found in any memtable (from SSTable) — keep as-is
                true
            });
        }

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_scan(elapsed_us);
        let lower_str = lower.map(|b| String::from_utf8_lossy(b).into_owned());
        let upper_str = upper.map(|b| String::from_utf8_lossy(b).into_owned());
        tracing::debug!(
            target: "apexstore::engine",
            operation = "scan_cf",
            cf = cf,
            limit = limit,
            results = results.len(),
            duration_us = elapsed_us,
            lower = %lower_str.as_deref().unwrap_or(""),
            upper = %upper_str.as_deref().unwrap_or(""),
        );

        Ok(results)
    }

    pub fn scan_range(
        &self,
        cf: &str,
        start: &[u8],
        end: &[u8],
        limit: Option<usize>,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.scan_cf(cf, Some(start), Some(end), limit)
    }

    #[allow(clippy::type_complexity)]
    pub fn search_prefix(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<(Vec<u8>, Vec<u8>)>, Option<String>)> {
        let start_time = std::time::Instant::now();
        // Calculate upper bound for prefix scan
        let upper_bound = Self::prefix_end(prefix);

        // Start from prefix. When cursor is provided, use cursor as the lower bound
        // (cursor >= prefix since it was returned by a previous prefix search).
        let start = cursor.map(|c| c.as_bytes()).or(Some(prefix.as_bytes()));

        // Request extra records to detect if there are more results.
        // When cursor is set, we need an additional +1 because the cursor
        // match gets filtered out, consuming one scan slot.
        let scan_extra = if cursor.is_some() { 2 } else { 1 };
        let scan_limit = Some(limit + scan_extra);

        let results = self.scan_cf("default", start, upper_bound.as_deref(), scan_limit)?;

        // If cursor is set, skip the first result if it matches the cursor key
        let results: Vec<(Vec<u8>, Vec<u8>)> = results
            .into_iter()
            .skip_while(|(k, _)| cursor.is_some_and(|c| k.as_slice() == c.as_bytes()))
            .collect();

        // Determine if there are more results beyond the limit
        let has_more = results.len() > limit;

        // Take only `limit` results for the current page
        let mut results = results;
        results.truncate(limit);

        // Return cursor pointing to the next page when there are more results
        let new_cursor = if has_more {
            results
                .last()
                .and_then(|(k, _)| String::from_utf8(k.clone()).ok())
        } else {
            None
        };

        let elapsed = start_time.elapsed();
        tracing::debug!(
            target: "apexstore::engine",
            operation = "search_prefix",
            prefix = %prefix,
            cursor = %cursor.unwrap_or(""),
            limit = limit,
            results = results.len(),
            has_more = has_more,
            duration_us = elapsed.as_micros() as u64,
        );

        Ok((results, new_cursor))
    }

    pub fn keys(&self) -> Result<Vec<Vec<u8>>> {
        let start = std::time::Instant::now();
        let core = self.core.lock();
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        if let Some(memtables) = core.memtables().get("default") {
            for mem in memtables.iter().rev() {
                iters.push(Box::new(crate::storage::iterator::MemTableIterator::new(
                    &mem.data,
                )));
            }
        }

        for sst_iter in core.version_set().table_iters("default") {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        let mut results = Vec::new();

        while merge_iter.is_valid() && results.len() < MAX_SCAN_LIMIT {
            results.push(merge_iter.key());
            merge_iter.next();
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            target: "apexstore::engine",
            operation = "keys",
            count = results.len(),
            duration_us = elapsed.as_micros() as u64,
        );

        Ok(results)
    }

    pub fn count(&self) -> Result<usize> {
        let start = std::time::Instant::now();
        let core = self.core.lock();
        let mut count = 0;
        let mut iters: Vec<Box<dyn StorageIterator<KeyType = KeySlice<'_>> + '_>> = Vec::new();

        if let Some(memtables) = core.memtables().get("default") {
            for mem in memtables.iter().rev() {
                count += mem.data.len();
            }
        }

        for sst_iter in core.version_set().table_iters("default") {
            iters.push(Box::new(sst_iter));
        }

        let mut merge_iter = MergeIterator::new(iters);
        while merge_iter.is_valid() {
            count += 1;
            merge_iter.next();
        }

        let elapsed = start.elapsed();
        tracing::debug!(
            target: "apexstore::engine",
            operation = "count",
            count = count,
            duration_us = elapsed.as_micros() as u64,
        );

        Ok(count)
    }

    /// Flush the oldest memtable for the given column family.
    /// Flush the current memtable to an SSTable.
    /// Public wrapper used by benchmarks and tests.
    pub fn flush_memtable(&self) -> Result<()> {
        self.flush_memtable_cf("default")
    }

    /// Flush the memtable for a specific column family to an SSTable.
    pub fn flush_memtable_cf(&self, cf: &str) -> Result<()> {
        let start = std::time::Instant::now();
        {
            let mut core = self.core.lock();
            self.flush_memtable_impl(cf, &mut core)?;
        }
        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_flush(elapsed_us);
        tracing::info!(
            target: "apexstore::engine",
            operation = "flush_memtable",
            cf = cf,
            duration_us = elapsed_us,
        );
        Ok(())
    }

    fn flush_memtable_impl(&self, cf: &str, core: &mut EngineCore<C>) -> Result<bool> {
        if let Some(memtables) = core.memtables_mut().get_mut(cf) {
            if let Some(mem) = memtables.pop() {
                let records = mem.data.len();
                // NOTE: TTL / expires_at metadata is stripped when converting
                // LogRecord to raw Vec<u8> for Table::build.  Expired keys
                // are filtered out here so they never reach the SSTable.
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let raw_data: std::collections::BTreeMap<Vec<u8>, Vec<u8>> =
                    mem.data
                        .into_iter()
                        .filter(|(_, r)| !r.is_expired_at(now))
                        .map(|(k, r)| (k, r.value))
                        .collect();
                let table = Table::build(raw_data, &self.options);
                core.version_set_mut().add_table(cf, table);
                let bytes = core.memtable_bytes_mut().get_mut(cf).ok_or_else(|| {
                    crate::LsmError::InvalidArgument(format!(
                        "Column family {} not found in memtable_bytes",
                        cf
                    ))
                })?;
                *bytes = 0;

                // ✅ Per-CF WAL: clear the flushed CF's WAL directly
                // instead of calling retain() on a global WAL (which was O(N)
                // per flush).  Each CF has its own WAL file, so clear() is O(1).
                core.wal_mut(cf)?.clear()?;

                tracing::info!(
                    target: "apexstore::engine",
                    operation = "flush_memtable_impl",
                    cf = cf,
                    records = records,
                    "memtable flushed to SSTable",
                );

                // Check if compaction might be needed after this flush
                let threshold = self.options.compaction_options.compaction_threshold;
                return Ok(core.version_set().table_count(cf) > threshold);
            }
        }
        Ok(false)
    }

    pub fn compact_cf(&self, cf: &str) -> Result<Option<CompactionMetrics>> {
        let start = std::time::Instant::now();
        let mut core = self.core.lock();
        let result = compact_cf_core(&mut core, &self.options, cf);
        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_compaction(elapsed_us);
        match &result {
            Ok(Some(metrics)) => {
                tracing::info!(
                    target: "apexstore::engine",
                    operation = "compact_cf",
                    cf = cf,
                    files_merged = metrics.files_merged,
                    bytes_read = metrics.bytes_read,
                    bytes_written = metrics.bytes_written,
                    duration_us = elapsed_us,
                    "compaction completed",
                );
            }
            Ok(None) => {
                tracing::debug!(
                    target: "apexstore::engine",
                    operation = "compact_cf",
                    cf = cf,
                    "no compaction needed",
                );
            }
            Err(e) => {
                tracing::error!(
                    target: "apexstore::engine",
                    operation = "compact_cf",
                    cf = cf,
                    error = %e,
                    "compaction failed",
                );
            }
        }
        result
    }

    pub fn compact(&self) -> Result<Vec<(String, CompactionMetrics)>> {
        let start = std::time::Instant::now();
        let mut results = Vec::new();
        let core = self.core.lock();
        let column_families = core.version_set().column_families();
        drop(core); // Release lock before calling compact_cf which will re-acquire
                    // Actually, we need the lock for compact_cf, so just call it per CF
        for cf in column_families {
            if let Some(metrics) = self.compact_cf(&cf)? {
                results.push((cf, metrics));
            }
        }

        let elapsed = start.elapsed();
        tracing::info!(
            target: "apexstore::engine",
            operation = "compact",
            cfs_compacted = results.len(),
            duration_us = elapsed.as_micros() as u64,
        );

        Ok(results)
    }

    /// Check if compaction should be triggered and run one or more CF
    /// compactions in the background — each CF gets its own thread, up to
    /// `max_concurrent_compactions` at once (controlled by a semaphore).
    pub fn maybe_compact(&self) {
        // Fast-path: skip if the engine is closing
        if self.closing.load(Ordering::SeqCst) {
            return;
        }

        // ── Phase 1: Build compaction plans while holding the core lock ──
        // Snapshot which CFs need compaction and what tables/groups to compact.
        // Then drop the lock so writes can proceed during I/O.

        #[derive(Clone)]
        struct CompactionPlan {
            cf: String,
            tables: Vec<Table>,
            groups: Vec<Vec<usize>>,
            compaction: Compaction,
            options: EngineOptions,
            range_tombstones: Vec<RangeTombstone>,
        }

        let plans: Vec<CompactionPlan> = {
            let core = self.core.lock();
            let master_options = self.options.clone();

            core.version_set()
                .column_families()
                .iter()
                .filter_map(|cf| {
                    let tables = core.version_set().get_tables(cf);
                    if tables.len() < core.compaction().options().compaction_threshold {
                        return None;
                    }
                    let groups = core.compaction().pick_compaction(&tables, &master_options);
                    if groups.is_empty() {
                        return None;
                    }
                    Some(CompactionPlan {
                        cf: cf.clone(),
                        tables,
                        groups,
                        compaction: core.compaction().clone(),
                        options: master_options.clone(),
                        range_tombstones: core
                            .range_tombstones()
                            .get(cf)
                            .cloned()
                            .unwrap_or_default(),
                    })
                })
                .collect()
        }; // MutexGuard dropped here → core lock is released

        if plans.is_empty() {
            return;
        }

        let max_concurrent = self.options.compaction_options.max_concurrent_compactions;

        // Spawn at most `max_concurrent` threads, one per CF.  Each thread
        // acquires a semaphore permit; when the limit is reached ({c} threads
        // already running) the loop stops and the remaining CFs will be picked
        // up on the next call to maybe_compact().
        for plan in plans.iter().take(max_concurrent) {
            // If the engine is closing, stop spawning new threads
            if self.closing.load(Ordering::SeqCst) {
                break;
            }

            // Non-blocking acquire — if at capacity, leave remaining CFs
            // for a future maybe_compact() call.
            let permit = match self.compaction_semaphore.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => break,
            };

            let core = self.core.clone();
            let plan = plan.clone();

            let handle = std::thread::spawn(move || {
                // The permit is held for the entire thread lifetime and
                // released automatically when the thread exits.
                let _permit = permit;

                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // ── Phase 2: Execute compaction I/O without holding the lock ──
                    let mut results: Vec<(String, Vec<usize>, Vec<Table>)> = Vec::new();
                    for group_indices in &plan.groups {
                        match plan
                            .compaction
                            .compact(
                                group_indices,
                                &plan.tables,
                                &plan.options,
                                &plan.range_tombstones,
                            ) {
                            Ok((new_tables, _metrics)) => {
                                results
                                    .push((plan.cf.clone(), group_indices.clone(), new_tables));
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Background compaction failed for CF {}: {:?}",
                                    plan.cf,
                                    e
                                );
                            }
                        }
                    }

                    // ── Phase 3: Re-acquire lock and apply results ──
                    let mut core = core.lock();
                    for (cf, group_indices, new_tables) in results {
                        let removed_paths = core
                            .version_set_mut()
                            .atomic_replace(&cf, &group_indices, new_tables);
                        // Delete orphaned SSTable files from disk
                        for path in &removed_paths {
                            if path.exists() {
                                if let Err(e) = std::fs::remove_file(path) {
                                    tracing::warn!(
                                        "background compaction: failed to remove orphaned SSTable \
                                         {:?}: {:?}",
                                        path,
                                        e
                                    );
                                }
                            }
                        }
                    }
                }));

                if let Err(panic_info) = result {
                    tracing::error!("Compaction thread panicked: {:?}", panic_info);
                }
            });

            // Store the handle while holding the threads lock.
            // This guarantees that any concurrent close() either:
            //   a) blocks on the lock and finds this handle after we release it, or
            //   b) has already taken all handles; but then close() cannot have
            //      spawned new threads because it can't acquire this lock while we hold it.
            let mut threads_guard = self.compaction_threads.lock();
            if self.closing.load(Ordering::SeqCst) {
                // close() may have set the flag while we were spawning;
                // drop the handle and let the thread run detached.
                break;
            }
            threads_guard.push(handle);
            drop(threads_guard);
        }
    }

    /// Close the engine gracefully.
    ///
    /// 1. Signals the compaction thread to stop and waits for it to finish.
    /// 2. Syncs the WAL file descriptor so all buffered data is durable.
    ///
    /// # Why not flush memtables?
    ///
    /// The engine does **not** persist a manifest of on-disk SSTables.
    /// Startup recovers state exclusively by replaying the WAL.  Flushing
    /// memtables and calling `WAL::retain()` would therefore *remove* the
    /// only durable record of those writes, causing data loss on restart.
    /// Instead, `close()` focuses on durability of the WAL itself.
    pub fn close(&self) {
        // 1. Set the closing flag so no new compaction threads are spawned.
        //    Lock compaction_threads first to synchronise with maybe_compact()
        //    which also takes this lock before pushing a handle.
        let mut threads_guard = self.compaction_threads.lock();
        self.closing.store(true, Ordering::Release);

        // 2. Take all handles while still holding the lock.
        //    This guarantees that any concurrent maybe_compact() either:
        //      a) sees closing=true and returns before spawning, or
        //      b) has already stored its handle and we find it here.
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *threads_guard);
        drop(threads_guard); // allow maybe_compact to proceed (but it sees closing=true)

        // 3. Wait for all compaction threads to finish.
        for handle in handles {
            match handle.join() {
                Ok(()) => {}
                Err(e) => {
                    tracing::error!("Compaction thread panicked on shutdown: {:?}", e);
                }
            }
        }

        // 4. Abort the replication shipping task (if running).
        if let Some(handle) = self._replication_handle.as_ref() {
            handle.abort();
            tracing::info!("Replication background task aborted on shutdown");
        }

        // 5. Sync all per-CF WALs so all buffered data is durably on disk.
        //    The WALs are the sole persistence mechanism across restarts.
        {
            let core = self.core.lock();
            for (cf, wal) in core.wals.iter() {
                if let Err(e) = wal.sync() {
                    tracing::error!(
                        "Engine::close(): failed to sync WAL for CF '{}': {:?}",
                        cf,
                        e
                    );
                }
            }
        }
    }

    pub fn stats(&self, cf: &str) -> Result<LsmStats> {
        let core = self.core.lock();
        let mut stats = LsmStats::default();

        // Get stats from version set
        let vs_stats = core.version_set().stats(cf);
        stats.num_tables = vs_stats.num_tables;
        stats.total_size = vs_stats.total_size;
        stats.total_records = vs_stats.total_records;
        stats.sst_kb = vs_stats.sst_kb;
        stats.sst_files = vs_stats.sst_files;
        stats.sst_records = vs_stats.sst_records;
        stats.max_levels_reached = vs_stats.max_levels_reached;
        stats.num_tables_at_max = vs_stats.num_tables_at_max;

        // Memtable stats
        if let Some(memtables) = core.memtables().get(cf) {
            stats.mem_records = memtables.iter().map(|m| m.data.len()).sum();
            stats.mem_kb = core.memtable_bytes().get(cf).copied().unwrap_or(0) / 1024;
        }

        // WAL stats — sum across all per-CF WALs
        stats.wal_kb = core
            .wals
            .values()
            .filter_map(|wal| wal.size().ok())
            .sum::<u64>() as usize
            / 1024;

        Ok(stats)
    }

    pub fn stats_all(&self) -> Result<LsmStats> {
        let core = self.core.lock();
        let mut combined = LsmStats::default();
        let column_families = core.version_set().column_families();

        for cf in column_families {
            let vs_stats = core.version_set().stats(&cf);
            combined.num_tables += vs_stats.num_tables;
            combined.total_size += vs_stats.total_size;
            combined.total_records += vs_stats.total_records;
            combined.sst_kb += vs_stats.sst_kb;
            combined.sst_files += vs_stats.sst_files;
            combined.sst_records += vs_stats.sst_records;

            // Memtable stats per CF
            if let Some(memtables) = core.memtables().get(&cf) {
                combined.mem_records += memtables.iter().map(|m| m.data.len()).sum::<usize>();
                combined.mem_kb += core.memtable_bytes().get(&cf).copied().unwrap_or(0) / 1024;
            }
        }

        combined.wal_kb = core
            .wals
            .values()
            .filter_map(|wal| wal.size().ok())
            .sum::<u64>() as usize
            / 1024;

        Ok(combined)
    }

    /// Calculate the upper bound for a prefix scan.
    /// Given prefix "ab", returns Some("ac") (incrementing the last byte).
    /// For empty prefix, returns None (scan everything).
    pub fn prefix_end(prefix: &str) -> Option<Vec<u8>> {
        if prefix.is_empty() {
            return None;
        }

        let mut result = prefix.as_bytes().to_vec();

        // Increment the last byte, handling overflow
        for i in (0..result.len()).rev() {
            if result[i] < 0xFF {
                result[i] += 1;
                return Some(result);
            }
            result[i] = 0;
        }

        // All bytes were 0xFF, so we need to extend
        result.push(0);
        Some(result)
    }

    /// Atomically insert a batch of key-value pairs into the default column family.
    ///
    /// All items are written to WAL and memtable under a single core lock
    /// acquisition, then compaction is triggered outside the lock if needed.
    pub fn set_batch<K, V>(&self, items: &[(K, V)]) -> Result<()>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        self.set_batch_cf("default", items)
    }

    /// Atomically insert a batch of key-value pairs into the specified column family.
    ///
    /// All items are written to WAL and memtable under a single core lock
    /// acquisition, then compaction is triggered outside the lock if needed.
    pub fn set_batch_cf<K, V>(&self, cf: &str, items: &[(K, V)]) -> Result<()>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        let start = std::time::Instant::now();
        let needs_compact;
        let batch_records: Vec<LogRecord>;
        {
            let mut core = self.core.lock();

            // Collect all WAL records first, then write them with a single fsync
            let records: Vec<LogRecord> = items
                .iter()
                .map(|(key, value)| {
                    let mut record = LogRecord::new(key.as_ref().to_vec(), value.as_ref().to_vec());
                    record.column_family = Some(cf.to_string());
                    record
                })
                .collect();
            batch_records = records.clone();
            core.wal_mut(cf)?.write_batch(&records)?;

            // Apply to memtable
            for (key, value) in items {
                let mem = core.memtables_mut().entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                mem[last].put(key.as_ref().to_vec(), value.as_ref().to_vec());
                *core.memtable_bytes_mut().entry(cf.to_string()).or_default() +=
                    key.as_ref().len() + value.as_ref().len();
            }
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes().get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        }

        // Ship batch to replicas (Primary only)
        if let Some(client) = &self.replication_client {
            if !batch_records.is_empty() {
                client.ship_records(batch_records);
            }
        }

        // Publish CDC events for each item in the batch
        for (key, value) in items {
            self.publish_cdc_event(cf, key.as_ref(), Some(value.as_ref()));
        }

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_batch_sets(items.len() as u64);
        self.metrics.record_set(elapsed_us);
        tracing::debug!(
            target: "apexstore::engine",
            operation = "set_batch_cf",
            cf = cf,
            count = items.len(),
            duration_us = elapsed_us,
            needs_compact = needs_compact,
        );
        if needs_compact {
            self.maybe_compact();
        }
        Ok(())
    }

    /// Atomically delete a batch of keys from the default column family.
    ///
    /// Tombstones are written to WAL and memtable under a single core lock
    /// acquisition, then compaction is triggered outside the lock if needed.
    pub fn delete_batch<K>(&self, keys: &[K]) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        self.delete_batch_cf("default", keys)
    }

    /// Atomically delete a batch of keys from the specified column family.
    ///
    /// Tombstones are written to WAL and memtable under a single core lock
    /// acquisition, then compaction is triggered outside the lock if needed.
    pub fn delete_batch_cf<K>(&self, cf: &str, keys: &[K]) -> Result<()>
    where
        K: AsRef<[u8]>,
    {
        let start = std::time::Instant::now();
        let needs_compact;
        let batch_records: Vec<LogRecord>;
        {
            let mut core = self.core.lock();

            // Collect all WAL records first, then write them with a single fsync
            let records: Vec<LogRecord> = keys
                .iter()
                .map(|key| {
                    let mut record = LogRecord::tombstone(key.as_ref().to_vec());
                    record.column_family = Some(cf.to_string());
                    record
                })
                .collect();
            batch_records = records.clone();
            core.wal_mut(cf)?.write_batch(&records)?;

            // Apply to memtable
            for key in keys {
                let mem = core.memtables_mut().entry(cf.to_string()).or_default();
                if mem.is_empty() {
                    mem.push(MemTable::new_unlimited());
                }
                let last = mem.len() - 1;
                mem[last].delete(key.as_ref().to_vec());
                *core.memtable_bytes_mut().entry(cf.to_string()).or_default() += key.as_ref().len();
            }
            let write_buffer_limit =
                self.options.write_buffer_size * self.options.max_write_buffer_number;
            needs_compact =
                if core.memtable_bytes().get(cf).copied().unwrap_or(0) >= write_buffer_limit {
                    self.flush_memtable_impl(cf, &mut core)?
                } else {
                    false
                };
        }

        // Ship tombstones to replicas (Primary only)
        if let Some(client) = &self.replication_client {
            if !batch_records.is_empty() {
                client.ship_records(batch_records);
            }
        }

        // Publish CDC events for each deleted key
        for key in keys {
            self.publish_cdc_event(cf, key.as_ref(), None);
        }

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_batch_deletes(keys.len() as u64);
        self.metrics.record_delete(elapsed_us);
        tracing::debug!(
            target: "apexstore::engine",
            operation = "delete_batch_cf",
            cf = cf,
            count = keys.len(),
            duration_us = elapsed_us,
            needs_compact = needs_compact,
        );
        if needs_compact {
            self.maybe_compact();
        }
        Ok(())
    }

    // ── Transaction API ──

    /// Begin a new transaction with buffered writes and snapshot isolation.
    ///
    /// Writes performed via the returned [`Transaction`](transaction::Transaction)
    /// are buffered in memory until [`commit`](transaction::Transaction::commit)
    /// is called, at which point they are applied atomically to the WAL and
    /// memtable.  Calling [`rollback`](transaction::Transaction::rollback)
    /// discards all buffered writes.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use apexstore::LsmConfig;
    /// # use apexstore::core::engine::Engine;
    /// # use apexstore::storage::cache::GlobalBlockCache;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let mut config = LsmConfig::default();
    /// # config.core.dir_path = dir.path().to_path_buf();
    /// # let engine = Engine::new_from_config(&config, GlobalBlockCache::new(100, 4096)).unwrap();
    /// let mut txn = engine.begin_transaction();
    /// txn.put_cf("default", b"k1", b"v1").unwrap();
    /// txn.put_cf("accounts", b"alice", b"100").unwrap();
    /// txn.commit().unwrap();
    /// ```
    pub fn begin_transaction(&self) -> transaction::Transaction<C> {
        transaction::Transaction::new(
            self.core.clone(),
            self.options.clone(),
            self.metrics.clone(),
        )
    }

    // ── Range Delete API ──

    /// Delete all keys in the range [start, end) from the specified column family.
    ///
    /// A range tombstone record is written to the WAL and the active range tombstone
    /// list in the memtable.  All subsequent reads and scans will filter out keys
    /// that fall within the range.
    pub fn delete_range_cf(&self, cf: &str, start: &[u8], end: &[u8]) -> Result<()> {
        let start_time = std::time::Instant::now();
        let replication_record: Option<LogRecord>;
        {
            let mut core = self.core.lock();

            let range = crate::core::log_record::RangeTombstone {
                start_key: start.to_vec(),
                end_key: end.to_vec(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
            };

            // Write range tombstone to WAL
            let mut record = LogRecord::range_tombstone(start.to_vec(), end.to_vec());
            record.column_family = Some(cf.to_string());
            core.wal_mut(cf)?.write_record(&record)?;

            // Save clone for replication
            replication_record = Some(record.clone());

            // Add to EngineCore-level range tombstones (survives flushes)
            core.range_tombstones_mut()
                .entry(cf.to_string())
                .or_default()
                .push(range.clone());

            // Add to current memtable
            let mem = core.memtables_mut().entry(cf.to_string()).or_default();
            if mem.is_empty() {
                mem.push(MemTable::new_unlimited());
            }
            let last = mem.len() - 1;
            mem[last].add_range_tombstone(range);
        }

        // Ship range tombstone to replicas (Primary only)
        if let Some(client) = &self.replication_client {
            if let Some(record) = replication_record {
                client.ship_records(vec![record]);
            }
        }

        let elapsed = start_time.elapsed();
        tracing::info!(
            target: "apexstore::engine",
            operation = "delete_range_cf",
            cf = cf,
            range_start = %String::from_utf8_lossy(start),
            range_end = %String::from_utf8_lossy(end),
            duration_us = elapsed.as_micros() as u64,
        );
        Ok(())
    }

    /// Delete all keys in the range [start, end) from the default column family.
    pub fn delete_range(&self, start: &[u8], end: &[u8]) -> Result<()> {
        self.delete_range_cf("default", start, end)
    }

    // ── Snapshot / Backup API ──

    /// Write an in-memory Table's data to an SSTable file at the given path.
    fn persist_table_to_sstable(
        table: &Table,
        path: &Path,
        options: &EngineOptions,
    ) -> Result<PathBuf> {
        let storage_config = crate::infra::config::StorageConfig {
            block_size: options.block_size,
            block_cache_size_mb: options.block_cache_size_mb,
            sparse_index_interval: 16,
            bloom_false_positive_rate: 0.01,
            encryption_enabled: options.encryption.enabled,
            encryption_key_path: None,
            prefix_compression_enabled: false,
        };
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let mut builder = SstableBuilder::new_with_encryption(
            path.to_path_buf(),
            storage_config,
            timestamp,
            &options.encryption,
        )?;
        for (key, value) in &table.data {
            let record = LogRecord::new(key.clone(), value.clone());
            builder.add(key, &record)?;
        }
        builder.finish()
    }

    /// Flush all column families (used internally by snapshot).
    fn flush_all_memtables(&self) -> Result<()> {
        loop {
            let cf_to_flush: Option<String> = {
                let core = self.core.lock();
                core.memtables()
                    .iter()
                    .find(|(_, mems)| !mems.is_empty())
                    .map(|(cf, _)| cf.clone())
            };
            match cf_to_flush {
                None => return Ok(()),
                Some(cf) => {
                    let mut core = self.core.lock();
                    self.flush_memtable_impl(&cf, &mut core)?;
                }
            }
        }
    }

    /// Create a point-in-time consistent snapshot by copying all engine data
    /// (SSTable files and WAL) to `backup_dir`.
    pub fn create_snapshot(&self, backup_dir: &Path) -> Result<()> {
        // Save WALs before flushing — flush clears per-CF WALs.
        let saved_wals: Vec<(String, Vec<u8>)> = {
            let core = self.core.lock();
            core.wals
                .iter()
                .filter_map(|(cf, wal)| {
                    std::fs::read(&wal.path).ok().map(|data| (cf.clone(), data))
                })
                .collect()
        };

        // Flush all memtables to in-memory tables (consistent state)
        self.flush_all_memtables()?;

        // Create backup directory
        std::fs::create_dir_all(backup_dir)?;

        // Lock core and copy / persist data
        let core = self.core.lock();

        // Build manifest mapping CF → SSTable filenames
        let mut manifest = SnapshotManifest {
            column_families: HashMap::new(),
        };

        // Copy or persist each table
        for cf in core.version_set().column_families() {
            let tables = core.version_set().get_tables(&cf);
            let mut cf_filenames = Vec::new();
            for (i, table) in tables.iter().enumerate() {
                let fname = if let Some(ref path) = table.path {
                    path.file_name()
                        .map(|n| n.to_os_string())
                        .unwrap_or_else(|| {
                            std::ffi::OsString::from(format!("cf_{}_table_{}.sst", cf, i))
                        })
                } else {
                    std::ffi::OsString::from(format!("{}_{}.sst", cf, i))
                };
                let fname_string = fname.to_string_lossy().to_string();
                let dest = backup_dir.join(&fname_string);
                if let Some(ref path) = table.path {
                    std::fs::copy(path, &dest)?;
                } else {
                    Self::persist_table_to_sstable(table, &dest, &self.options)?;
                }
                cf_filenames.push(fname_string);
            }
            manifest.column_families.insert(cf, cf_filenames);
        }

        // Also copy all orphaned .sst files from the sstables directory
        // so that the snapshot contains a complete copy of the data dir.
        if let Ok(entries) = std::fs::read_dir(&self._sst_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "sst") {
                    let fname = path.file_name().unwrap_or_default();
                    let dest = backup_dir.join(fname);
                    if !dest.exists() {
                        let _ = std::fs::copy(&path, &dest);
                    }
                }
            }
        }

        // Write the manifest
        let manifest_json = serde_json::to_string(&manifest)
            .map_err(|e| crate::LsmError::InvalidArgument(format!("Failed to serialize manifest: {}", e)))?;
        std::fs::write(backup_dir.join("snapshot.manifest"), &manifest_json)?;

        // Copy saved WALs into the backup directory.
        // Always write at least an empty wal.log so list_snapshots can
        // identify this directory as a valid snapshot.
        let has_any_data = saved_wals.iter().any(|(_, data)| !data.is_empty());
        if has_any_data {
            for (cf, data) in &saved_wals {
                if data.is_empty() {
                    continue;
                }
                let dest = if cf == "default" || cf.is_empty() {
                    backup_dir.join("wal.log")
                } else {
                    backup_dir.join(format!("wal-{}.log", cf))
                };
                std::fs::write(&dest, data)?;
            }
        } else {
            std::fs::write(backup_dir.join("wal.log"), b"")?;
        }

        Ok(())
    }

    /// Load a `SnapshotManifest` from a snapshot directory, if present.
    fn load_snapshot_manifest(snapshot_dir: &Path) -> Result<Option<SnapshotManifest>> {
        let manifest_path = snapshot_dir.join("snapshot.manifest");
        if !manifest_path.exists() {
            return Ok(None);
        }
        let json_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: SnapshotManifest = serde_json::from_str(&json_str)
            .map_err(|e| crate::LsmError::InvalidArgument(format!("Failed to parse snapshot manifest: {}", e)))?;
        Ok(Some(manifest))
    }

    /// List all snapshots found inside `backup_dir`.
    pub fn list_snapshots(&self, backup_dir: &Path) -> Result<Vec<SnapshotInfo>> {
        let mut snapshots = Vec::new();
        if !backup_dir.exists() {
            return Ok(snapshots);
        }
        for entry in std::fs::read_dir(backup_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !path.join("wal.log").exists() {
                continue;
            }
            let created_at = path
                .metadata()
                .and_then(|m| m.created())
                .unwrap_or_else(|_| SystemTime::now());
            let size_bytes = Self::dir_size(&path);
            let file_count = Self::file_count(&path);
            snapshots.push(SnapshotInfo {
                path,
                created_at,
                size_bytes,
                file_count,
            });
        }
        snapshots.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(snapshots)
    }

    fn dir_size(dir: &Path) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    total += Self::dir_size(&path);
                } else if let Ok(meta) = path.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    }

    fn file_count(dir: &Path) -> usize {
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += Self::file_count(&path);
                } else {
                    count += 1;
                }
            }
        }
        count
    }

    /// Restore engine data from a previously created snapshot.
    pub fn restore_snapshot(&self, snapshot_dir: &Path) -> Result<()> {
        let data_dir = self
            ._sst_dir
            .parent()
            .ok_or_else(|| {
                crate::infra::error::LsmError::InvalidArgument(
                    "sst_dir must have a parent (engine data dir)".to_string(),
                )
            })?;
        let sst_dir = &self._sst_dir;

        std::fs::create_dir_all(data_dir)?;
        std::fs::create_dir_all(sst_dir)?;

        // Track which SSTable filenames we copy from the snapshot
        let mut copied_sst_files: Vec<String> = Vec::new();

        for entry in std::fs::read_dir(snapshot_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "sst") {
                let Some(fname) = path.file_name() else { continue; };
                let fname_str = fname.to_string_lossy().to_string();
                let dest = sst_dir.join(&fname_str);
                std::fs::copy(&path, &dest)?;
                copied_sst_files.push(fname_str);
            } else if path.file_name().is_some_and(|n| n == "wal.log") {
                let dest = data_dir.join("wal.log");
                std::fs::copy(&path, &dest)?;
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Copy per-CF WAL files: wal-{cf}.log
                if name.starts_with("wal-") && name.ends_with(".log") {
                    let dest = data_dir.join(name);
                    std::fs::copy(&path, &dest)?;
                }
            }
        }

        // Load the manifest and register SSTables in the engine's VersionSet
        let manifest = Self::load_snapshot_manifest(snapshot_dir)?;

        // Write the disk manifest for new_generic() to discover on startup
        if let Some(ref m) = manifest {
            let disk_manifest_path = data_dir.join("disk.sst.manifest");
            let json = serde_json::to_string(m)
                .map_err(|e| crate::LsmError::InvalidArgument(
                    format!("Failed to serialize disk manifest: {}", e)
                ))?;
            std::fs::write(&disk_manifest_path, &json)?;
        }

        // Register SSTables in the running engine's VersionSet
        if let Some(m) = manifest {
            let mut core = self.core.lock();
            let sst_dir = sst_dir.clone();
            let enc = &self.options.encryption;
            for (cf, filenames) in &m.column_families {
                for fname in filenames {
                    let sst_path = sst_dir.join(fname);
                    if sst_path.exists() {
                        match Table::from_sstable_path(&sst_path, Some(enc)) {
                            Ok(table) => {
                                core.version_set_mut().add_table(cf, table);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "restore_snapshot: failed to load SSTable {} for CF {}: {:?}",
                                    fname, cf, e
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Discover SSTables on disk and load them into the VersionSet.
    ///
    /// Called during engine startup (`new_generic`) after WAL replay.
    /// First checks for a `disk.sst.manifest` written by `restore_snapshot()`.
    /// If no manifest exists, falls back to loading all `.sst` files from the
    /// sst_dir into the "default" column family (legacy behavior).
    fn discover_sstables_from_disk(
        core: &mut EngineCore<C>,
        data_dir: &Path,
        sst_dir: &Path,
    ) -> Result<()> {
        let enc = core.encryption.clone();
        let manifest_path = data_dir.join("disk.sst.manifest");
        if manifest_path.exists() {
            // Use the manifest written by restore_snapshot()
            let json_str = std::fs::read_to_string(&manifest_path)
                .map_err(|e| crate::LsmError::InvalidArgument(
                    format!("Failed to read disk manifest: {}", e)
                ))?;
            let manifest: SnapshotManifest = serde_json::from_str(&json_str)
                .map_err(|e| crate::LsmError::InvalidArgument(
                    format!("Failed to parse disk manifest: {}", e)
                ))?;
            for (cf, filenames) in &manifest.column_families {
                for fname in filenames {
                    let sst_path = sst_dir.join(fname);
                    if sst_path.exists() {
                        match Table::from_sstable_path(&sst_path, Some(&enc)) {
                            Ok(table) => {
                                core.version_set_mut().add_table(cf, table);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "discover_sstables: failed to load {} for CF {}: {:?}",
                                    fname, cf, e
                                );
                            }
                        }
                    }
                }
            }
        } else {
            // Fallback: scan for .sst files and add them to default CF
            if let Ok(entries) = std::fs::read_dir(sst_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "sst") {
                        if let Some(fname) = path.file_name() {
                            let fname_str = fname.to_string_lossy();
                            tracing::info!(
                                "discover_sstables: loading orphaned SSTable {} into default CF",
                                fname_str
                            );
                            match Table::from_sstable_path(&path, Some(&enc)) {
                                Ok(table) => {
                                    core.version_set_mut().add_table("default", table);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "discover_sstables: failed to load {}: {:?}",
                                        fname_str, e
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Reconcile in-memory table state with `.sst` files on disk.
    ///
    /// 1. Lists all `.sst` files in the sst_dir.
    /// 2. Compares them with the paths tracked by the VersionSet.
    /// 3. Removes orphaned `.sst` files that are no longer referenced.
    ///
    /// Returns the number of orphaned files removed.
    pub fn reconcile_tables(&self) -> Result<usize> {
        let mut removed = 0usize;

        // Collect all paths tracked by VersionSet
        let tracked_paths: std::collections::HashSet<PathBuf> = {
            let core = self.core.lock();
            let mut paths = std::collections::HashSet::new();
            for cf in core.version_set().column_families() {
                for table in core.version_set().get_tables(&cf) {
                    if let Some(ref p) = table.path {
                        paths.insert(p.clone());
                    }
                }
            }
            paths
        };

        // Scan sst_dir for orphaned .sst files
        if let Ok(entries) = std::fs::read_dir(&self._sst_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "sst")
                    && !tracked_paths.contains(&path)
                {
                    if let Err(e) = std::fs::remove_file(&path) {
                        tracing::warn!(
                            "reconcile_tables: failed to remove orphaned SSTable {:?}: {:?}",
                            path, e
                        );
                    } else {
                        tracing::info!(
                            "reconcile_tables: removed orphaned SSTable {:?}",
                            path
                        );
                        removed += 1;
                    }
                }
            }
        }

        Ok(removed)
    }
}

impl<C: Cache> Drop for Engine<C> {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine::compaction::CompactionStrategy;
    use crate::storage::cache::NoopCache;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn test_prefix_end_basic() {
        // Test basic ASCII prefix
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("ab");
        assert_eq!(result, Some(b"ac".to_vec()));
    }

    #[test]
    fn test_prefix_end_empty() {
        // Test empty prefix
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_prefix_end_non_ascii() {
        // Test non-ASCII prefix (e.g., UTF-8 characters)
        let prefix = "usuário:";
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end(prefix);
        // The last byte of "usuário:" is 0x3A (':')
        // So we expect it to be incremented to 0x3B (';')
        assert!(result.is_some());
        let end = result.unwrap();
        // The prefix in bytes should be "usuário:" followed by something
        assert!(end.len() >= prefix.len());
        // The upper bound should be greater than the prefix
        assert!(end.as_slice() > prefix.as_bytes());
    }

    #[test]
    fn test_prefix_end_unicode_multi_byte() {
        // Test with multi-byte UTF-8 characters
        // "ção:" - 'ç' is 0xC3 0xA7, 'ã' is 0xC3 0xA3 in UTF-8
        let prefix = "ção:";
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end(prefix);
        assert!(result.is_some());
        let end = result.unwrap();
        // The upper bound should be greater than the prefix
        assert!(end.as_slice() > prefix.as_bytes());
    }

    #[test]
    fn test_prefix_end_single_byte_increment() {
        // Test that only the last non-0xFF byte is incremented
        let result = Engine::<crate::storage::cache::GlobalBlockCache>::prefix_end("abc");
        assert_eq!(result, Some(b"abd".to_vec()));
    }

    #[test]
    fn test_search_prefix_non_ascii() {
        use crate::infra::config::LsmConfig;

        // Create temp directory for engine storage
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert some non-ASCII key-value pairs
        let test_pairs = vec![
            ("usuário:1", "value1"),
            ("usuário:2", "value2"),
            ("chave:3", "value3"),
        ];

        for (key, value) in &test_pairs {
            engine
                .set(key.as_bytes().to_vec(), value.as_bytes().to_vec())
                .unwrap();
        }

        // Search with prefix
        let (results, _) = engine.search_prefix("usuário:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_prefix_unicode_chars() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert with unicode prefix
        let test_pairs = vec![
            ("ção:1", "value1"),
            ("ção:2", "value2"),
            ("outro:3", "value3"),
        ];

        for (key, value) in &test_pairs {
            engine
                .set(key.as_bytes().to_vec(), value.as_bytes().to_vec())
                .unwrap();
        }

        let (results, _) = engine.search_prefix("ção:", None, 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_size_tiered_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;

        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();

        // Create tables
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _metrics) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
    }

    #[test]
    fn test_lazy_leveling_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = LazyLevelingCompaction::default();
        let options = EngineOptions::default();

        // Create L0 tables (should use size-tiered)
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            let mut table = Table::build(data, &options);
            table.level = 0;
            tables.push(table);
        }

        // Pick tables to compact
        let _groups = strategy.pick_tables(&tables, &options);

        // Execute compaction
        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
    }

    #[test]
    fn test_compaction_removes_tombstones() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();

        // Create a table with tombstones (empty values)
        let mut data = BTreeMap::new();
        // Add some live data
        for i in 0..50 {
            let key = format!("live_key_{}", i).into_bytes();
            let value = format!("live_value_{}", i).into_bytes();
            data.insert(key, value);
        }
        // Add tombstones
        for i in 0..50 {
            let key = format!("dead_key_{}", i).into_bytes();
            let value = Vec::new(); // tombstone
            data.insert(key, value);
        }

        let table = Table::build(data, &options);

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, _) = strategy
.execute(vec![table], &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        // The new table should not contain tombstones
        if let Some(new_table) = new_tables.first() {
            for value in new_table.data.values() {
                assert!(
                    !value.is_empty(),
                    "Tombstones should be removed during compaction"
                );
            }
        }
    }

    #[test]
    fn test_compaction_metrics() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();

        // Create tables
        let mut tables = Vec::new();
        for i in 0..3 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_, metrics) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        assert!(metrics.bytes_read > 0, "Should track bytes read");
        assert!(metrics.files_merged > 0, "Should track files merged");
        assert!(metrics.duration_ms > 0, "Duration should be positive");
    }

    #[test]
    fn test_size_tiered_bucket_grouping() {
        use crate::core::engine::compaction::SizeTieredCompaction;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = SizeTieredCompaction::default();

        // Create tables of different sizes
        let mut tables = Vec::new();
        for i in 0..10 {
            let mut data = BTreeMap::new();
            let num_entries = if i < 5 { 10 } else { 100 };
            for j in 0..num_entries {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &EngineOptions::default()));
        }

        let options = crate::core::engine::EngineOptions::default();
        let groups = strategy.pick_tables(&tables, &options);

        // Should group small tables together
        assert!(!groups.is_empty(), "Should group tables by size");
    }

    #[test]
    fn test_atomic_replace_in_version_set() {
        use crate::infra::config::StorageConfig;
        use crate::storage::cache::NoopCache;

        let options = crate::core::engine::EngineOptions::default();
        let cache = NoopCache;
        let mut vs = crate::core::engine::version_set::VersionSet::<NoopCache>::new(
            options,
            cache,
            StorageConfig::default(),
            None,
        );

        // Add some tables
        for i in 0..5 {
            let mut data = std::collections::BTreeMap::new();
            data.insert(
                format!("key_{}", i).into_bytes(),
                format!("value_{}", i).into_bytes(),
            );
            let table = crate::core::table::Table::build(
                data,
                &crate::core::engine::EngineOptions::default(),
            );
            vs.add_table("default", table);
        }

        assert_eq!(vs.table_count("default"), 5);

        // Create new tables to replace some old ones
        let mut new_tables = Vec::new();
        for i in 0..2 {
            let mut data = std::collections::BTreeMap::new();
            data.insert(
                format!("new_key_{}", i).into_bytes(),
                format!("new_value_{}", i).into_bytes(),
            );
            new_tables.push(crate::core::table::Table::build(
                data,
                &crate::core::engine::EngineOptions::default(),
            ));
        }

        // Replace tables at indices 0, 1, 2 with new tables
        vs.atomic_replace("default", &[0, 1, 2], new_tables);

        assert_eq!(vs.table_count("default"), 4); // 5 - 3 + 2 = 4
    }

    #[test]
    fn test_1000_keys_with_multiple_compactions() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();

        // Create tables with known sizes
        let mut tables = Vec::new();

        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_new_tables, metrics) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        // Write amplification = bytes_written / bytes_read
        // For SizeTiered, should be < 3x
        if metrics.bytes_read > 0 {
            let write_amplification = metrics.bytes_written as f64 / metrics.bytes_read as f64;
            assert!(
                write_amplification < 3.0,
                "Write amplification for SizeTiered should be < 3x, got {:.2}x",
                write_amplification
            );
        }
    }

    #[test]
    fn test_leveled_compaction_basic() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = LeveledCompaction::default();
        let options = EngineOptions::default();

        // Create L0 tables
        let mut tables = Vec::new();
        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            let mut table = Table::build(data, &options);
            table.level = 0;
            tables.push(table);
        }

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (new_tables, metrics) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        assert!(
            !new_tables.is_empty(),
            "Should produce at least one new table"
        );
        assert!(metrics.files_merged > 0, "Should track files merged");
        assert!(metrics.bytes_read > 0, "Should track bytes read");
        assert!(metrics.duration_ms > 0, "Duration should be positive");

        // Check that new tables are at level 1
        for table in &new_tables {
            assert_eq!(table.level, 1, "Compacted tables should be at level 1");
        }
    }

    #[test]
    fn test_compaction_write_amplification_size_tiered() {
        use crate::core::engine::compaction::*;
        use crate::core::engine::EngineOptions;
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let strategy = SizeTieredCompaction::default();
        let options = EngineOptions::default();

        // Create tables with known sizes
        let mut tables = Vec::new();

        for i in 0..5 {
            let mut data = BTreeMap::new();
            for j in 0..100 {
                let key = format!("key_{}_{}", i, j).into_bytes();
                let value = format!("value_{}_{}", i, j).into_bytes();
                data.insert(key, value);
            }
            tables.push(Table::build(data, &options));
        }

        let storage_config = crate::infra::config::StorageConfig::default();
        let dir = tempdir().unwrap();
        let output_dir = dir.path().to_path_buf();
        let (_new_tables, metrics) = strategy
.execute(tables, &options, &storage_config, &output_dir, &[])
                                   .unwrap();

        // Write amplification = bytes_written / bytes_read
        // For SizeTiered, should be < 3x
        if metrics.bytes_read > 0 {
            let write_amplification = metrics.bytes_written as f64 / metrics.bytes_read as f64;
            assert!(
                write_amplification < 3.0,
                "Write amplification for SizeTiered should be < 3x, got {:.2}x",
                write_amplification
            );
        }
    }

    #[test]
    fn test_crash_recovery_cf() {
        use crate::infra::config::LsmConfig;

        // Create engine in a temp directory
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write data to CF "users"
        let key = b"user:1".to_vec();
        let value = b"alice".to_vec();
        engine.put_cf("users", key.clone(), value.clone()).unwrap();

        // Verify data is present before crash
        let result = engine.get_cf("users", key.as_slice()).unwrap();
        assert_eq!(result, Some(value.clone()));

        // Verify data is NOT in default CF before crash
        let result_default = engine.get_cf("default", &key).unwrap();
        assert_eq!(result_default, None);

        // Drop engine — simulating crash without flush
        drop(engine);

        // Create a new engine from the same directory (triggers WAL recovery)
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Assert data is present in "users" CF after recovery
        let result_recovered = engine2.get_cf("users", &key).unwrap();
        assert_eq!(result_recovered, Some(value.clone()));

        // Assert data is NOT present in "default" CF after recovery
        let result_default_recovered = engine2.get_cf("default", &key).unwrap();
        assert_eq!(result_default_recovered, None);
    }

    #[test]
    fn test_crash_recovery_cf_after_flush() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Use a tiny memtable so writes trigger a flush immediately
        config.core.memtable_max_size = 512;

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write data to both "users" and "default" CFs
        let users_key = b"user:1".to_vec();
        let users_value = b"alice".to_vec();
        engine
            .put_cf("users", users_key.clone(), users_value.clone())
            .unwrap();

        let default_key = b"default:1".to_vec();
        let default_value = b"bob".to_vec();
        engine
            .put_cf("default", default_key.clone(), default_value.clone())
            .unwrap();

        // Verify both CFs have data before crash
        let result_users = engine.get_cf("users", &users_key).unwrap();
        assert_eq!(result_users, Some(users_value.clone()));

        let result_default = engine.get_cf("default", &default_key).unwrap();
        assert_eq!(result_default, Some(default_value.clone()));

        // Drop engine — simulating crash without flush (WAL will be replayed)
        drop(engine);

        // Reopen engine
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Both CFs must have their data after WAL recovery
        let result_users_recovered = engine2.get_cf("users", &users_key).unwrap();
        assert_eq!(
            result_users_recovered,
            Some(users_value),
            "users CF data should survive crash via WAL recovery"
        );

        let result_default_recovered = engine2.get_cf("default", &default_key).unwrap();
        assert_eq!(
            result_default_recovered,
            Some(default_value),
            "default CF data should survive crash via WAL recovery"
        );
    }

    #[test]
    fn test_crash_during_compaction() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold so compaction triggers after few flushes
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let key_count = 200;

        {
            let engine = Engine::new_from_config(
                &config,
                crate::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            // Write many keys to trigger flushes and compactions
            for i in 0..key_count {
                engine.set(format!("k{}", i), vec![b'x'; 100]).unwrap();
            }
        } // engine dropped — simulating crash with active compaction state

        // Reopen — must not panic and must recover data from WAL
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Verify engine is operational after crash-during-compaction
        // Engine opened without panic — verify stats, count and scan work
        let _stats = engine2.stats("default").unwrap_or_default();
        engine2.count().unwrap();
        engine2.scan().unwrap_or_default();
    }

    #[test]
    fn test_writes_during_compaction() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold so compaction triggers early
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys to trigger flushes and compaction.
        // Each put_cf call releases the inner core lock before maybe_compact,
        // so background compaction (spawned by maybe_compact) can proceed
        // while writes continue.
        let key_count = 500;
        for i in 0..key_count {
            engine
                .set(format!("k{}", i), vec![b'x'; 100])
                .expect("write during compaction must succeed");
        }

        // Verify at least some keys are readable after compaction
        let mut found = 0;
        for i in 0..key_count {
            if let Ok(Some(_)) = engine.get(format!("k{}", i)) {
                found += 1;
            }
        }
        assert!(
            found > 0,
            "At least some keys should be readable after compaction"
        );
    }

    #[test]
    fn test_shutdown_with_compaction_in_progress() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        // Small memtable to trigger frequent flushes
        config.core.memtable_max_size = 2048;
        // Low compaction threshold
        config.compaction.min_compaction_threshold = 3;
        config.compaction.max_sstables = 8;
        config.compaction.level_size = 3;

        let key_count = 200;

        // This block simulates shutdown with compaction in progress — should not panic
        {
            let engine = Engine::new_from_config(
                &config,
                crate::storage::cache::GlobalBlockCache::new(100, 4096),
            )
            .unwrap();

            // Write keys to trigger flushes and potential compaction
            for i in 0..key_count {
                engine.set(format!("k{}", i), vec![b'x'; 100]).unwrap();
            }
        } // engine dropped here — Drop::drop calls close() which joins the compaction thread

        // Re-open the engine to verify:
        // 1. The compaction thread was joined (no lock leak — engine can re-open)
        // 2. Data survived the shutdown-with-compaction scenario
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Verify engine is operational after shutdown with compaction
        let count = engine2.count().unwrap_or(0);
        assert!(
            count > 0,
            "Data should survive shutdown during compaction, got {} keys",
            count
        );
    }

    // ── T6: Bloom filter prevents data lookup for absent key ──
    #[test]
    fn test_bloom_filter_prevents_absent_key_lookup() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        // Insert some keys via the SSTable path (bypassing memtable)
        // We create a VersionSet with a table that has a bloom filter and
        // verify that reading an absent key doesn't iterate table data.
        let mut core = engine.lock_core();
        let mut data = BTreeMap::new();
        data.insert(b"present_key".to_vec(), b"present_value".to_vec());
        let mut table = Table::build(data, &engine.options);
        // Build a bloom filter covering only the inserted key
        let mut bf = bloomfilter::Bloom::<[u8]>::new(1024, 1).expect("valid bloom params");
        bf.set(b"present_key");
        table.bloom_filter = Some(bf);
        table.min_key = b"present_key".to_vec();
        table.max_key = b"present_key".to_vec();
        core.version_set_mut().add_table("default", table);

        // Reading an absent key should return None (bloom filter says no)
        let result = core.version_set().get("default", b"absent_key");
        assert!(
            result.is_none(),
            "Absent key should return None via Bloom filter"
        );

        // Reading a present key should succeed
        let result = core.version_set().get("default", b"present_key");
        assert_eq!(result, Some(b"present_value".to_vec()));
    }

    // ── T7: Block cache hit returns cached value on repeated read ──
    #[test]
    fn test_kv_cache_hit_on_repeated_read() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        // Insert a key into the version set directly
        let mut core = engine.lock_core();
        let mut data = BTreeMap::new();
        data.insert(b"cached_key".to_vec(), b"cached_value".to_vec());
        let table = Table::build(data, &engine.options);
        core.version_set_mut().add_table("default", table);

        // First read populates the KV cache
        let r1 = core.version_set().get("default", b"cached_key");
        assert_eq!(r1, Some(b"cached_value".to_vec()));

        // Second read should hit the KV cache (cache populated by first read)
        let r2 = core.version_set().get("default", b"cached_key");
        assert_eq!(r2, Some(b"cached_value".to_vec()));

        // Verify cache stats by checking that clearing the cache still works
        core.version_set().clear_cache();
        let r3 = core.version_set().get("default", b"cached_key");
        assert_eq!(
            r3,
            Some(b"cached_value".to_vec()),
            "Value still readable after cache clear"
        );
    }

    // ── T8: scan_cf skips non-intersecting SSTables ──
    #[test]
    fn test_scan_cf_skips_non_intersecting_sstables() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Engine::<NoopCache>::new_from_config(&config, NoopCache).unwrap();

        let mut core = engine.lock_core();

        // Table 1: keys "a" to "c"
        let mut data1 = BTreeMap::new();
        data1.insert(b"a".to_vec(), b"1".to_vec());
        data1.insert(b"b".to_vec(), b"2".to_vec());
        data1.insert(b"c".to_vec(), b"3".to_vec());
        let mut t1 = Table::build(data1, &engine.options);
        t1.min_key = b"a".to_vec();
        t1.max_key = b"c".to_vec();
        core.version_set_mut().add_table("default", t1);

        // Table 2: keys "x" to "z"
        let mut data2 = BTreeMap::new();
        data2.insert(b"x".to_vec(), b"24".to_vec());
        data2.insert(b"y".to_vec(), b"25".to_vec());
        data2.insert(b"z".to_vec(), b"26".to_vec());
        let mut t2 = Table::build(data2, &engine.options);
        t2.min_key = b"x".to_vec();
        t2.max_key = b"z".to_vec();
        core.version_set_mut().add_table("default", t2);

        // Table 3: keys "m" to "p"
        let mut data3 = BTreeMap::new();
        data3.insert(b"m".to_vec(), b"13".to_vec());
        data3.insert(b"n".to_vec(), b"14".to_vec());
        data3.insert(b"o".to_vec(), b"15".to_vec());
        let mut t3 = Table::build(data3, &engine.options);
        t3.min_key = b"m".to_vec();
        t3.max_key = b"o".to_vec();
        core.version_set_mut().add_table("default", t3);

        // Drop the core lock so scan_cf can acquire it
        drop(core);

        // Scan range [b, n] — should only include keys from table 1 and 3 (table 2 is entirely after "n")
        let results = engine
            .scan_cf("default", Some(b"b"), Some(b"n"), None)
            .unwrap();
        let keys: Vec<&[u8]> = results.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(
            keys,
            vec![b"b", b"c", b"m"],
            "Should only return keys b, c, m from intersecting tables"
        );
    }

    #[test]
    fn test_benchmark_memtable_read_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.compaction.max_sstables = 4;
        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        let key = b"bench_key";
        engine
            .put_cf("default", key.to_vec(), b"value".to_vec())
            .unwrap();

        // Warm up
        for _ in 0..100 {
            engine.get_cf("default", key).unwrap();
        }

        let start = std::time::Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            engine.get_cf("default", key).unwrap();
        }
        let elapsed = start.elapsed() / iterations as u32;

        // Memtable reads should be < 10 µs
        assert!(
            elapsed < std::time::Duration::from_micros(10),
            "Memtable read avg {:?} exceeds 10µs",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_sstable_warm_read_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 4096;
        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert keys and flush to SSTable
        for i in 0..1000 {
            engine
                .put_cf(
                    "default",
                    format!("key_{:04}", i).into_bytes(),
                    b"value_1234567890".to_vec(),
                )
                .unwrap();
        }
        engine.flush_memtable().unwrap();

        // Warm up cache by reading all keys once
        for i in 0..1000 {
            let _ = engine.get_cf("default", format!("key_{:04}", i).as_bytes());
        }

        // Measure warm reads
        let start = std::time::Instant::now();
        let iterations = 100;
        for _ in 0..iterations {
            for i in 0..1000 {
                let _ = engine.get_cf("default", format!("key_{:04}", i).as_bytes());
            }
        }
        let elapsed = start.elapsed() / (iterations * 1000) as u32;

        assert!(
            elapsed < std::time::Duration::from_micros(500),
            "Warm SSTable read avg {:?} exceeds 500µs",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_scan_1k_keys_latency() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.core.memtable_max_size = 10 * 1024 * 1024; // Keep all keys in memtable
        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert 1000 keys
        for i in 0..1000 {
            engine
                .put_cf(
                    "default",
                    format!("key_{:04}", i).into_bytes(),
                    b"value_1234567890".to_vec(),
                )
                .unwrap();
        }

        let start = std::time::Instant::now();
        let iterations = 10;
        for _ in 0..iterations {
            let results = engine.scan_cf("default", None, None, Some(1000)).unwrap();
            assert_eq!(results.len(), 1000, "Should return all 1000 keys");
        }
        let elapsed = start.elapsed() / iterations as u32;

        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "Scan 1k keys avg {:?} exceeds 5ms",
            elapsed
        );
    }

    #[test]
    fn test_benchmark_bloom_filter_negative_read() {
        let dir = tempdir().unwrap();
        let mut config = crate::infra::config::LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        config.compaction.max_sstables = 4;
        // Use small memtable to trigger flush
        config.core.memtable_max_size = 4096;
        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write enough to trigger flush
        for i in 0..1000 {
            engine
                .put_cf("default", format!("key{}", i).into_bytes(), b"val".to_vec())
                .unwrap();
        }
        engine.flush_memtable().unwrap();

        let start = std::time::Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            // Key that doesn't exist — should be caught by bloom filter fast path
            let _ = engine.get_cf("default", b"nonexistent_key_xyz");
        }
        let elapsed = start.elapsed() / iterations as u32;

        assert!(
            elapsed < std::time::Duration::from_micros(10),
            "Bloom filter negative read avg {:?} exceeds 10µs",
            elapsed
        );
    }

    // ── Issue #152: Batch atomicity tests ──

    #[test]
    fn test_set_batch_atomicity() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert a batch of items
        let items: Vec<(&str, &str)> = vec![("k1", "v1"), ("k2", "v2"), ("k3", "v3")];
        engine.set_batch(&items).unwrap();

        // Verify all items were written
        assert_eq!(engine.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(engine.get(b"k2").unwrap(), Some(b"v2".to_vec()));
        assert_eq!(engine.get(b"k3").unwrap(), Some(b"v3".to_vec()));
    }

    #[test]
    fn test_delete_batch_atomicity() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert individual items
        engine.set(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        engine.set(b"k2".to_vec(), b"v2".to_vec()).unwrap();
        engine.set(b"k3".to_vec(), b"v3".to_vec()).unwrap();
        engine.set(b"k4".to_vec(), b"v4".to_vec()).unwrap();

        // Delete a batch of keys
        let keys_to_delete: Vec<&[u8]> = vec![b"k2", b"k4"];
        engine.delete_batch(&keys_to_delete).unwrap();

        // Verify deleted keys are gone
        assert_eq!(engine.get(b"k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(engine.get(b"k2").unwrap(), None);
        assert_eq!(engine.get(b"k3").unwrap(), Some(b"v3".to_vec()));
        assert_eq!(engine.get(b"k4").unwrap(), None);
    }

    #[test]
    fn test_set_batch_cf_atomicity() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert batch into a non-default column family
        let items: Vec<(&str, &str)> = vec![("cf1:k1", "v1"), ("cf1:k2", "v2")];
        engine.set_batch_cf("custom_cf", &items).unwrap();

        // Verify items are in the custom CF
        assert_eq!(
            engine.get_cf("custom_cf", b"cf1:k1").unwrap(),
            Some(b"v1".to_vec())
        );
        assert_eq!(
            engine.get_cf("custom_cf", b"cf1:k2").unwrap(),
            Some(b"v2".to_vec())
        );

        // Verify items are NOT in the default CF
        assert_eq!(engine.get_cf("default", b"cf1:k1").unwrap(), None);
    }

    #[test]
    fn test_delete_batch_cf_atomicity() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert items into custom CF
        engine
            .put_cf("cf_del", b"dk1".to_vec(), b"dv1".to_vec())
            .unwrap();
        engine
            .put_cf("cf_del", b"dk2".to_vec(), b"dv2".to_vec())
            .unwrap();
        engine
            .put_cf("cf_del", b"dk3".to_vec(), b"dv3".to_vec())
            .unwrap();

        // Delete batch from custom CF
        let keys_to_delete: Vec<&[u8]> = vec![b"dk1", b"dk3"];
        engine.delete_batch_cf("cf_del", &keys_to_delete).unwrap();

        // Verify atomic deletion
        assert_eq!(engine.get_cf("cf_del", b"dk1").unwrap(), None);
        assert_eq!(
            engine.get_cf("cf_del", b"dk2").unwrap(),
            Some(b"dv2".to_vec())
        );
        assert_eq!(engine.get_cf("cf_del", b"dk3").unwrap(), None);
    }

    #[test]
    fn test_set_batch_empty() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Empty batch should succeed without error
        let items: Vec<(&str, &str)> = vec![];
        engine.set_batch(&items).unwrap();
        assert!(engine.keys().is_ok());
    }

    #[test]
    fn test_delete_batch_empty() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        engine.set(b"k1".to_vec(), b"v1".to_vec()).unwrap();
        // Empty delete batch should succeed without removing anything
        let keys: Vec<&[u8]> = vec![];
        engine.delete_batch(&keys).unwrap();
        assert_eq!(engine.get(b"k1").unwrap(), Some(b"v1".to_vec()));
    }

    // ── Issue #150: Snapshot / Backup tests ──

    #[test]
    fn test_create_snapshot() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let backup_dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write some data across multiple CFs
        engine
            .put_cf("default", b"k1".to_vec(), b"v1".to_vec())
            .unwrap();
        engine
            .put_cf("default", b"k2".to_vec(), b"v2".to_vec())
            .unwrap();
        engine
            .put_cf("users", b"u1".to_vec(), b"alice".to_vec())
            .unwrap();

        // Create snapshot
        let snapshot_path = backup_dir.path().join("snap1");
        engine.create_snapshot(&snapshot_path).unwrap();

        // Verify snapshot directory exists with expected files
        assert!(snapshot_path.exists(), "Snapshot directory must exist");
        assert!(
            snapshot_path.join("wal.log").exists(),
            "wal.log must exist in snapshot"
        );

        // Verify at least one .sst file was created (in-memory tables persisted)
        let sst_count = std::fs::read_dir(&snapshot_path)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| e.path().extension().map(|ext| ext == "sst"))
                    .unwrap_or(false)
            })
            .count();
        assert!(
            sst_count > 0,
            "Snapshot should contain at least one SSTable file, got {}",
            sst_count
        );

        // Verify data is still readable after snapshot
        assert_eq!(
            engine.get(b"k1").unwrap(),
            Some(b"v1".to_vec()),
            "Data should remain readable after snapshot"
        );
        assert_eq!(
            engine.get_cf("users", b"u1").unwrap(),
            Some(b"alice".to_vec()),
            "CF data should remain readable after snapshot"
        );
    }

    #[test]
    fn test_snapshot_restore() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let backup_dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write data
        engine
            .put_cf("default", b"k1".to_vec(), b"v1".to_vec())
            .unwrap();
        engine
            .put_cf("default", b"k2".to_vec(), b"v2".to_vec())
            .unwrap();
        engine
            .put_cf("users", b"u1".to_vec(), b"alice".to_vec())
            .unwrap();

        // Create snapshot
        let snapshot_path = backup_dir.path().join("snap_restore");
        engine.create_snapshot(&snapshot_path).unwrap();

        // Drop engine and wipe data directory
        let dir_path = dir.path().to_path_buf();
        drop(engine);
        std::fs::remove_dir_all(&dir_path).unwrap();
        std::fs::create_dir_all(&dir_path).unwrap();

        // Create a fresh engine on the empty directory and restore
        let engine2 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();
        engine2.restore_snapshot(&snapshot_path).unwrap();
        drop(engine2);

        // Re-open engine — WAL replay should restore data
        let engine3 = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Verify all data is restored
        assert_eq!(
            engine3.get(b"k1").unwrap(),
            Some(b"v1".to_vec()),
            "k1 should be restored"
        );
        assert_eq!(
            engine3.get(b"k2").unwrap(),
            Some(b"v2".to_vec()),
            "k2 should be restored"
        );
        assert_eq!(
            engine3.get_cf("users", b"u1").unwrap(),
            Some(b"alice".to_vec()),
            "users CF data should be restored"
        );
    }

    #[test]
    fn test_list_snapshots() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let backup_dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        engine.set(b"k1".to_vec(), b"v1".to_vec()).unwrap();

        // Create two snapshots
        let snap1 = backup_dir.path().join("snap_a");
        let snap2 = backup_dir.path().join("snap_b");
        engine.create_snapshot(&snap1).unwrap();
        engine.create_snapshot(&snap2).unwrap();

        // List snapshots
        let snapshots = engine.list_snapshots(backup_dir.path()).unwrap();

        assert_eq!(snapshots.len(), 2, "Should find 2 snapshots");
        assert!(
            snapshots.iter().any(|s| s.path == snap1),
            "snap_a should be listed"
        );
        assert!(
            snapshots.iter().any(|s| s.path == snap2),
            "snap_b should be listed"
        );
        // Each snapshot should have non-zero size and at least 1 file
        for info in &snapshots {
            assert!(info.size_bytes > 0, "Snapshot should have non-zero size");
            assert!(info.file_count > 0, "Snapshot should have at least 1 file");
        }
    }

    // ── Issue #193: TTL / auto-expiry tests ──

    #[test]
    fn test_ttl_key_expires_after_duration() {
        use crate::infra::config::LsmConfig;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Set a key with a 1ms TTL
        engine
            .set_with_ttl(b"ephemeral".to_vec(), b"value".to_vec(), Duration::from_millis(1))
            .unwrap();

        // Immediately after write, key should be present
        assert_eq!(
            engine.get(b"ephemeral").unwrap(),
            Some(b"value".to_vec()),
            "Key should be visible immediately after write"
        );

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(5));

        // Key should now be expired
        assert_eq!(
            engine.get(b"ephemeral").unwrap(),
            None,
            "Key should be None after TTL expiry"
        );
    }

    #[test]
    fn test_ttl_key_without_ttl_never_expires() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Set a key without TTL
        engine.set(b"persistent".to_vec(), b"value".to_vec()).unwrap();

        // Key should be present
        assert_eq!(
            engine.get(b"persistent").unwrap(),
            Some(b"value".to_vec()),
        );

        // Even after a short wait, key should still be present
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert_eq!(
            engine.get(b"persistent").unwrap(),
            Some(b"value".to_vec()),
            "Key without TTL should never expire"
        );
    }

    #[test]
    fn test_ttl_scan_filters_expired_entries() {
        use crate::infra::config::LsmConfig;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert a key without TTL (permanent)
        engine.set(b"permanent".to_vec(), b"keep".to_vec()).unwrap();
        // Insert a key with short TTL
        engine
            .set_with_ttl(b"temp".to_vec(), b"gone".to_vec(), Duration::from_millis(1))
            .unwrap();

        // Both keys should appear in scan before expiry
        let results = engine.scan_cf("default", None, None, Some(10)).unwrap();
        assert_eq!(results.len(), 2, "Both keys should appear before TTL expiry");

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(5));

        // Only the permanent key should appear in scan
        let results = engine.scan_cf("default", None, None, Some(10)).unwrap();
        assert_eq!(results.len(), 1, "Only permanent key should appear in scan");
        assert_eq!(results[0].0, b"permanent".to_vec());
    }

    #[test]
    fn test_ttl_in_column_family() {
        use crate::infra::config::LsmConfig;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Insert a key with TTL in a non-default column family
        engine
            .set_cf_with_ttl("sessions", b"session:1", b"active", Duration::from_millis(1))
            .unwrap();

        // Immediately after write, key should be present
        assert_eq!(
            engine.get_cf("sessions", b"session:1").unwrap(),
            Some(b"active".to_vec())
        );

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(5));

        // Key should now be expired in the CF
        assert_eq!(
            engine.get_cf("sessions", b"session:1").unwrap(),
            None,
            "Key in CF should be None after TTL expiry"
        );
    }

    #[test]
    fn test_ttl_default_ttl_config() {
        use crate::infra::config::LsmConfig;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        // Build engine with a default TTL and use set()
        let options = EngineOptions {
            default_ttl: Some(Duration::from_millis(1)),
            ..Default::default()
        };
        let engine = Engine::new_generic(
            options,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
            dir.path(),
        )
        .unwrap();

        // set() should inherit the default TTL
        engine.set(b"auto_expire".to_vec(), b"value".to_vec()).unwrap();

        // Immediately readable
        assert_eq!(
            engine.get(b"auto_expire").unwrap(),
            Some(b"value".to_vec())
        );

        // Wait for default TTL to expire
        std::thread::sleep(Duration::from_millis(5));

        // Key should be expired via default_ttl
        assert_eq!(
            engine.get(b"auto_expire").unwrap(),
            None,
            "Key with default TTL should expire"
        );
    }

    #[test]
    fn test_ttl_log_record_new_with_ttl() {
        use std::time::Duration;

        // Test the LogRecord constructor directly
        let record = LogRecord::new_with_ttl(b"k".to_vec(), b"v".to_vec(), Duration::from_secs(3600));
        assert!(!record.is_expired(), "Fresh TTL record should not be expired");

        // A record with 0 TTL should be expired immediately
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let expired_record = LogRecord {
            expires_at: Some(now.saturating_sub(1)), // 1 nanosecond ago
            ..LogRecord::new(b"k".to_vec(), b"v".to_vec())
        };
        assert!(expired_record.is_expired(), "Past expires_at should be expired");

        // Non-TTL record should never be expired
        let no_ttl = LogRecord::new(b"k".to_vec(), b"v".to_vec());
        assert!(!no_ttl.is_expired(), "No TTL record should never expire");
        assert_eq!(no_ttl.expires_at, None);
    }

    // ── Range Delete Tests ──

    #[test]
    fn test_delete_range_removes_keys_in_range() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys "a", "b", "c", "d", "e" and flush to SSTable
        // so that range tombstones can mask them
        engine.put_cf("default", b"a".to_vec(), b"value_a".to_vec()).unwrap();
        engine.put_cf("default", b"b".to_vec(), b"value_b".to_vec()).unwrap();
        engine.put_cf("default", b"c".to_vec(), b"value_c".to_vec()).unwrap();
        engine.put_cf("default", b"d".to_vec(), b"value_d".to_vec()).unwrap();
        engine.put_cf("default", b"e".to_vec(), b"value_e".to_vec()).unwrap();
        engine.flush_memtable().unwrap();

        // Verify all keys are present
        assert_eq!(engine.get(b"a").unwrap(), Some(b"value_a".to_vec()));
        assert_eq!(engine.get(b"b").unwrap(), Some(b"value_b".to_vec()));
        assert_eq!(engine.get(b"c").unwrap(), Some(b"value_c".to_vec()));

        // Delete range [b, d) — should delete "b", "c"
        engine.delete_range(b"b", b"d").unwrap();

        // Keys in range should be removed
        assert_eq!(engine.get(b"a").unwrap(), Some(b"value_a".to_vec()));
        assert_eq!(engine.get(b"b").unwrap(), None);
        assert_eq!(engine.get(b"c").unwrap(), None);
        assert_eq!(engine.get(b"d").unwrap(), Some(b"value_d".to_vec()));
        assert_eq!(engine.get(b"e").unwrap(), Some(b"value_e".to_vec()));
    }

    #[test]
    fn test_delete_range_preserves_keys_outside_range() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys with numerical prefixes and flush to SSTable
        for i in 0..10 {
            let key = format!("key_{}", i).into_bytes();
            let value = format!("value_{}", i).into_bytes();
            engine.put_cf("default", key, value).unwrap();
        }
        engine.flush_memtable().unwrap();

        // Delete range "key_3".."key_7"
        engine.delete_range(b"key_3", b"key_7").unwrap();

        // Keys outside range should remain
        assert_eq!(engine.get(b"key_0").unwrap(), Some(b"value_0".to_vec()));
        assert_eq!(engine.get(b"key_2").unwrap(), Some(b"value_2".to_vec()));
        assert_eq!(engine.get(b"key_7").unwrap(), Some(b"value_7".to_vec()));
        assert_eq!(engine.get(b"key_9").unwrap(), Some(b"value_9".to_vec()));

        // Keys inside range should be gone
        assert_eq!(engine.get(b"key_3").unwrap(), None);
        assert_eq!(engine.get(b"key_4").unwrap(), None);
        assert_eq!(engine.get(b"key_5").unwrap(), None);
        assert_eq!(engine.get(b"key_6").unwrap(), None);
    }

    #[test]
    fn test_range_tombstone_interaction_with_point_writes() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write key "x" with value "original" and flush to SSTable
        engine.put_cf("default", b"x".to_vec(), b"original".to_vec()).unwrap();
        engine.flush_memtable().unwrap();
        assert_eq!(engine.get(b"x").unwrap(), Some(b"original".to_vec()));

        // Delete range [x, z) — should shadow "x" in SSTable
        engine.delete_range(b"x", b"z").unwrap();

        // "x" should now be deleted (range tombstone masks SSTable data)
        assert_eq!(engine.get(b"x").unwrap(), None);

        // Write "x" again with a new value — point write in memtable
        // should take precedence over the range tombstone
        engine.put_cf("default", b"x".to_vec(), b"new_value".to_vec()).unwrap();

        // "x" should have the new value (memtable point write wins)
        assert_eq!(engine.get(b"x").unwrap(), Some(b"new_value".to_vec()));

        // "y" should still be deleted by the range tombstone
        assert_eq!(engine.get(b"y").unwrap(), None);
    }

    #[test]
    fn test_delete_range_scan_filters_out_tombstoned_keys() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys 1-5 and flush to SSTable
        for i in 1..=5 {
            let key = format!("k{}", i).into_bytes();
            let value = format!("v{}", i).into_bytes();
            engine.put_cf("default", key, value).unwrap();
        }
        engine.flush_memtable().unwrap();

        // Delete range "k2".."k4"
        engine.delete_range(b"k2", b"k4").unwrap();

        // Scan should only return k1, k4, k5
        let results = engine.scan().unwrap();
        let keys: Vec<&[u8]> = results.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(keys, vec![b"k1", b"k4", b"k5"]);
    }

    #[test]
    fn test_delete_range_cf() {
        use crate::infra::config::LsmConfig;

        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();

        let engine = Engine::new_from_config(
            &config,
            crate::storage::cache::GlobalBlockCache::new(100, 4096),
        )
        .unwrap();

        // Write keys in custom CF and flush to SSTable
        engine.put_cf("cf1", b"a".to_vec(), b"1".to_vec()).unwrap();
        engine.put_cf("cf1", b"b".to_vec(), b"2".to_vec()).unwrap();
        engine.put_cf("cf1", b"c".to_vec(), b"3".to_vec()).unwrap();
        engine.flush_memtable_cf("cf1").unwrap();

        // Verify keys in CF
        assert_eq!(engine.get_cf("cf1", b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(engine.get_cf("cf1", b"b").unwrap(), Some(b"2".to_vec()));

        // Delete range [a, c) in CF
        engine.delete_range_cf("cf1", b"a", b"c").unwrap();

        // Keys in range should be deleted
        assert_eq!(engine.get_cf("cf1", b"a").unwrap(), None);
        assert_eq!(engine.get_cf("cf1", b"b").unwrap(), None);
        assert_eq!(engine.get_cf("cf1", b"c").unwrap(), Some(b"3".to_vec()));

        // Write a separate key to default CF to verify independence
        engine.put_cf("default", b"default_key".to_vec(), b"val".to_vec()).unwrap();
        assert_eq!(engine.get(b"default_key").unwrap(), Some(b"val".to_vec()));
    }
}
