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
/// - **Infrastructure** (`Io`, `Codec`, `Time`) — low-level OS / serde
///   errors that are converted automatically via `#[from]`.
/// - **Storage format** (`InvalidSstableFormat`, `CorruptedData`,
///   `DecompressionFailed`, `WalCorruption`) — structural problems in
///   on-disk files.
/// - **Engine semantics** (`KeyNotFound`, `CompactionFailed`) — logical
///   errors arising from engine operations.
/// - **Configuration** (`Invalid*`, `ConfigValidation`) — parameter
///   validation failures raised at startup.
///
/// # Previous state → rationale for changes
///
/// The following variants were removed in this commit:
///
/// | Removed variant       | Reason |
/// |-----------------------|--------|
/// | `NotFound`            | Exact duplicate of `KeyNotFound` (same Display text, zero call sites) |
/// | `InvalidSstable`      | Context-free alias for `InvalidSstableFormat(String)` (zero call sites) |
/// | `LockPoisoned`        | `parking_lot` mutexes never poison; was unreachable dead code |
/// | `ConcurrentModification` | Zero call sites anywhere in the codebase |
/// | `SerializationFailed` | Zero call sites; superseded by `Codec` |
/// | `DeserializationFailed` | Zero call sites; superseded by `Codec` |
///
/// `Serialization` was renamed to `Codec` to match the `infra::codec`
/// module name and to avoid confusion between encode and decode paths.
#[derive(Error, Debug)]
pub enum LsmError {
    // -------------------------------------------------------------------------
    // Infrastructure errors — converted automatically via #[from]
    // -------------------------------------------------------------------------
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Covers both encode and decode failures from `bincode`.
    /// Converted automatically via `?` in `infra::codec`.
    #[error("Codec error: {0}")]
    Codec(#[from] bincode::Error),

    #[error("System time error: {0}")]
    Time(#[from] SystemTimeError),

    // -------------------------------------------------------------------------
    // Storage format errors
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
    #[error("Key not found")]
    KeyNotFound,

    #[error("Compaction failed: {0}")]
    CompactionFailed(String),

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

    #[error("Configuration validation failed: {0}")]
    ConfigValidation(String),
}

pub type Result<T> = std::result::Result<T, LsmError>;
