use bincode;
use std::io;
use std::time::SystemTimeError;
use thiserror::Error;

/// Unified error type for the ApexStore LSM engine.
///
/// # Design
///
/// Variants are grouped by origin:
///
/// - **Infrastructure** (`Io`, `Codec`, `JsonError`, `Time`) — low-level OS / serde
///   errors converted automatically via `#[from]`.
/// - **Storage format** (`InvalidSstableFormat`, `CorruptedData`,
///   `DecompressionFailed`, `WalCorruption`) — structural problems in on-disk files.
/// - **Engine semantics** (`NotFound`, `CompactionFailed`, `LockPoisoned`,
///   `ConcurrentModification`) — logical errors arising from engine operations.
/// - **Configuration** (`Invalid*`, `ConfigValidation`) — parameter
///   validation failures raised at startup.
///
/// # Variant history
///
/// See [`CHANGELOG.md`](https://github.com/ElioNeto/ApexStore/blob/main/CHANGELOG.md)
/// under `[Unreleased]` → `Removed` / `Changed` for the full history of removed
/// variants and renames (issue #92).
#[derive(Error, Debug)]
pub enum LsmError {
    // -------------------------------------------------------------------------
    // Infrastructure — converted automatically via #[from]
    // -------------------------------------------------------------------------
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Bincode encode/decode failures from `infra::codec`.
    #[error("Codec error: {0}")]
    Codec(#[from] bincode::Error),

    /// JSON encode/decode failures (serde_json), e.g. from `features::FeatureClient`.
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("System time error: {0}")]
    Time(#[from] SystemTimeError),

    // -------------------------------------------------------------------------
    // Storage format
    // -------------------------------------------------------------------------
    #[error("Invalid SSTable format: {0}")]
    InvalidSstableFormat(String),

    #[error("Corrupted data: {0}")]
    CorruptedData(String),

    #[error("Decompression failed: {0}")]
    DecompressionFailed(String),

    #[error("WAL corruption detected")]
    WalCorruption,

    // -------------------------------------------------------------------------
    // Engine semantics
    // -------------------------------------------------------------------------
    #[error("Key not found: {0}")]
    NotFound(String),

    #[error("Compaction failed: {0}")]
    CompactionFailed(String),

    /// Raised when a `std::sync::Mutex` is poisoned (i.e. a thread panicked
    /// while holding the lock). Not applicable to `parking_lot` mutexes.
    #[error("Lock poisoned: {0}")]
    LockPoisoned(&'static str),

    /// Raised in optimistic-concurrency retry loops when all attempts are
    /// exhausted (e.g. `FeatureClient::set_flag`).
    #[error("Concurrent modification conflict")]
    ConcurrentModification,

    // -------------------------------------------------------------------------
    // Request validation (runtime errors)
    // -------------------------------------------------------------------------
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    // -------------------------------------------------------------------------
    // Configuration validation
    // -------------------------------------------------------------------------
    #[error("Invalid block size: {0}")]
    InvalidBlockSize(String),

    #[error("Invalid cache size: {0}")]
    InvalidCacheSize(String),

    #[error("Invalid sparse index interval: {0}")]
    InvalidIndexInterval(String),

    #[error("Invalid Bloom filter false positive rate: {0}")]
    InvalidBloomRate(String),

    #[error("Invalid memtable size: {0}")]
    InvalidMemtableSize(String),

    #[error("Invalid compaction config: {0}")]
    InvalidCompactionConfig(String),

    #[error("Configuration validation failed: {0}")]
    ConfigValidation(String),
}

pub type Result<T> = std::result::Result<T, LsmError>;
