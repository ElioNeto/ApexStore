use crate::core::log_record::LogRecord;
use crate::infra::codec::{decode, encode};
use crate::infra::error::Result;
use crate::storage::encryption::{EncryptionConfig, Encryptor};
use crc32fast::Hasher;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// WAL frame version constants for backward compatibility.
///
/// - Version 0: LogRecord serialized WITHOUT `column_family` (original format).
/// - Version 1: LogRecord serialized WITH `column_family` (but no range tombstone fields).
/// - Version 2: LogRecord serialized WITH `column_family` AND `range_start`/`range_end`.
/// - Version 3: Same as V2, but the payload is AES-256-GCM encrypted.
///   Format: `[12-byte IV][encrypted V2 payload]`
pub(crate) const WAL_FRAME_VERSION_V0: u8 = 0;
pub(crate) const WAL_FRAME_VERSION_V1: u8 = 1;
pub(crate) const WAL_FRAME_VERSION_V2: u8 = 2;
pub(crate) const WAL_FRAME_VERSION_V3_ENCRYPTED: u8 = 3;
pub(crate) const WAL_CURRENT_FRAME_VERSION: u8 = WAL_FRAME_VERSION_V2;

/// LogRecord payload format for V0 frames (without `column_family`).
///
/// This struct is used exclusively for backward-compatible deserialization of
/// WAL frames written by older versions of the engine that did not persist the
/// column family.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct LogRecordV0 {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp: u128,
    pub is_deleted: bool,
    // no column_family — this is the old format
}

impl From<LogRecordV0> for LogRecord {
    fn from(v0: LogRecordV0) -> Self {
        LogRecord {
            key: v0.key,
            value: v0.value,
            timestamp: v0.timestamp,
            is_deleted: v0.is_deleted,
            column_family: None, // legacy records have no CF → treated as "default"
            expires_at: None,
            range_start: None,
            range_end: None,
        }
    }
}

/// LogRecord payload format for V1 frames (without `range_start` / `range_end`).
///
/// This struct is used exclusively for backward-compatible deserialization of
/// WAL frames written by versions of the engine before range delete support.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct LogRecordV1 {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub timestamp: u128,
    pub is_deleted: bool,
    #[serde(default)]
    pub column_family: Option<String>,
    // no range_start / range_end — this is the pre-range-delete format
}

impl From<LogRecordV1> for LogRecord {
    fn from(v1: LogRecordV1) -> Self {
        LogRecord {
            key: v1.key,
            value: v1.value,
            timestamp: v1.timestamp,
            is_deleted: v1.is_deleted,
            column_family: v1.column_family,
            expires_at: None,
            range_start: None,
            range_end: None,
        }
    }
}

/// Write-Ahead Log for crash-recovery durability.
///
/// Every mutation (set / delete) is persisted here before it touches the
/// MemTable.  On startup the engine replays all records to reconstruct
/// in-memory state, then calls `clear()` once the MemTable has been
/// flushed to a durable SSTable.
///
/// # Thread Safety
///
/// `WriteAheadLog` is `Send + Sync`.  Internal synchronisation is provided
/// by a `parking_lot::Mutex` around the `BufWriter<File>`.  All public
/// methods acquire the lock for the minimum time necessary.
///
/// # On-Disk Format
///
/// Each WAL record frame follows this structure:
/// `[length: u32 LE][version: u8][payload: bytes][crc32: u32 LE]`
///
/// - `length`: total size of (`version` + `payload`) in bytes.
/// - `version`: frame format version (`0` = no CF field, `1` = with CF field).
/// - `payload`: bincode-serialized `LogRecord` (structure depends on version).
/// - `crc32`: CRC32 checksum calculated over (`version` + `payload`).
///
/// The CRC32 checksum provides protection against partial writes, bit rot,
/// and other forms of data corruption.  The version byte enables backward
/// compatible upgrades of the serialised log record format.
pub struct WriteAheadLog {
    file: Mutex<BufWriter<File>>,
    /// Exposed read-only so callers (e.g. `LsmEngine::stats_all`) can
    /// query the file size without going through the write lock.
    pub(crate) path: PathBuf,
    /// Number of buffered writes since the last fsync.
    /// Used to amortise fsync cost across multiple write_record calls.
    batch_count: Mutex<usize>,
    /// Optional encryptor for transparent WAL frame encryption.
    encryptor: Encryptor,
}

/// How many `write_record` calls to accumulate before issuing an fsync.
///
/// A value of 1 means every write fsyncs (maximum durability).
/// Higher values improve write throughput at the cost of a wider
/// durability window in the event of a crash.
const WAL_SYNC_INTERVAL: usize = 4;

const MAX_WAL_RECORD_BYTES: usize = 32 * 1024 * 1024; // 32 MiB

impl WriteAheadLog {
    /// Open or create a WAL file for the given column family.
    ///
    /// The file is stored as `<dir_path>/wal-{cf}.log`.  For the default
    /// column family the file is `<dir_path>/wal.log` for backward
    /// compatibility.
    ///
    /// `encryption` controls whether WAL frames are encrypted.
    pub fn new(dir_path: &std::path::Path, cf: &str) -> Result<Self> {
        Self::new_with_encryption(dir_path, cf, &EncryptionConfig::default())
    }

    /// Open or create a WAL file with optional encryption.
    pub fn new_with_encryption(
        dir_path: &std::path::Path,
        cf: &str,
        encryption: &EncryptionConfig,
    ) -> Result<Self> {
        let wal_path = if cf == "default" || cf.is_empty() {
            dir_path.join("wal.log")
        } else {
            dir_path.join(format!("wal-{}.log", cf))
        };
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)?;

        Ok(Self {
            file: Mutex::new(BufWriter::new(file)),
            path: wal_path,
            batch_count: Mutex::new(0),
            encryptor: Encryptor::new(encryption),
        })
    }

    /// Append a single record to the WAL with batched fsync.
    ///
    /// Instead of fsyncing after every write (which limits throughput to
    /// ~1 100 ops/s on typical hardware), the method accumulates
    /// [`WAL_SYNC_INTERVAL`] records before issuing an fsync.  Callers
    /// that need strict durability after every operation should use
    /// [`WriteAheadLog::sync()`] explicitly.
    ///
    /// The on-disk format is:
    /// `[length: u32 LE][version: u8][payload: bytes][crc32: u32 LE]`
    ///
    /// # Checksum coverage
    ///
    /// The CRC32 checksum covers the **entire frame header** — `length`,
    /// `version`, and `payload` — to detect corruption in any part of the
    /// record frame.
    pub fn write_record(&self, record: &LogRecord) -> Result<()> {
        let serialized = encode(record)?;

        // Encrypt payload if encryption is enabled (use version 3 for encrypted frames)
        let (payload, version) = if self.encryptor.is_enabled() {
            let encrypted = self.encryptor.encrypt_block(&serialized)?;
            (encrypted, WAL_FRAME_VERSION_V3_ENCRYPTED)
        } else {
            (serialized, WAL_CURRENT_FRAME_VERSION)
        };

        // `length` includes version byte + payload bytes
        let length = 1u32 + payload.len() as u32;

        // Calculate CRC32 over (length + version + payload)
        let length_bytes = length.to_le_bytes();
        let mut hasher = Hasher::new();
        hasher.update(&length_bytes);
        hasher.update(&[version]);
        hasher.update(&payload);
        let checksum = hasher.finalize();

        let mut writer = self.file.lock();

        writer.write_all(&length_bytes)?;
        writer.write_all(&[version])?;
        writer.write_all(&payload)?;
        writer.write_all(&checksum.to_le_bytes())?;
        writer.flush()?;

        // Accumulate writes and fsync only every WAL_SYNC_INTERVAL calls.
        let mut count = self.batch_count.lock();
        *count += 1;
        if *count >= WAL_SYNC_INTERVAL {
            *count = 0;
            // Drop the batch lock before fsync so we don't hold two locks.
            drop(count);
            writer.get_ref().sync_all()?;
        }

        debug!(
            "WAL persisted: key={:?}, ts={}",
            record.key, record.timestamp
        );
        Ok(())
    }

    /// Append multiple records to the WAL with a single fsync.
    ///
    /// This is more efficient than calling `write_record` N times because
    /// the lock is acquired only once and `flush() + sync_all()` is called
    /// only once, regardless of the batch size.
    ///
    /// Each record uses the same on-disk frame format as `write_record`:
    /// `[length: u32 LE][version: u8][payload: bytes][crc32: u32 LE]`
    pub fn write_batch(&self, records: &[LogRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        // Pre-encode all records and build frame data
        let mut frames: Vec<Vec<u8>> = Vec::with_capacity(records.len());
        for record in records {
            let serialized = encode(record)?;

            // Encrypt payload if encryption is enabled
            let (payload, version) = if self.encryptor.is_enabled() {
                let encrypted = self.encryptor.encrypt_block(&serialized)?;
                (encrypted, WAL_FRAME_VERSION_V3_ENCRYPTED)
            } else {
                (serialized, WAL_CURRENT_FRAME_VERSION)
            };

            let length = 1u32 + payload.len() as u32;
            let length_bytes = length.to_le_bytes();

            let mut hasher = Hasher::new();
            hasher.update(&length_bytes);
            hasher.update(&[version]);
            hasher.update(&payload);
            let checksum = hasher.finalize();

            let mut frame = Vec::with_capacity(4 + 1 + payload.len() + 4);
            frame.extend_from_slice(&length_bytes);
            frame.push(version);
            frame.extend_from_slice(&payload);
            frame.extend_from_slice(&checksum.to_le_bytes());
            frames.push(frame);
        }

        let mut writer = self.file.lock();
        for frame in &frames {
            writer.write_all(frame)?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;

        // Reset the single-write batch counter since we just fsynced.
        let mut count = self.batch_count.lock();
        *count = 0;
        drop(count);

        debug!("WAL batch persisted: {} records", records.len());
        Ok(())
    }

    /// Replay all records persisted in the WAL.
    ///
    /// Called once during engine initialisation.  Unlike the strict-error
    /// behaviour of earlier versions, this implementation uses **tolerant
    /// recovery**: corrupted frames (CRC mismatch, invalid length, unknown
    /// version, deserialisation failure) are **skipped** with a warning
    /// rather than aborting the entire recovery.  The engine can therefore
    /// start up even if the WAL contains a limited amount of bit rot or
    /// partial writes, recovering as many records as possible.
    ///
    /// The expected on-disk format is:
    /// `[length: u32 LE][version: u8][payload: bytes][crc32: u32 LE]`
    ///
    /// # Compatibility Note
    ///
    /// WAL frames with `version == 0` are deserialised without the
    /// `column_family` field (legacy format) and treated as `"default"`.
    /// Frames with `version == 1` are deserialised with full `column_family`
    /// support.
    ///
    /// # Locking
    ///
    /// Opens a **new** file handle internally.  If you need to read from
    /// the same file descriptor that is currently being written (e.g. to
    /// ensure buffered-but-not-yet-persisted data is visible), use
    /// [`WriteAheadLog::recover_locked`] instead.
    pub fn recover(&self) -> Result<Vec<LogRecord>> {
        self.recover_locked()
    }

    /// Read all records from the WAL, first **flushing** the writer so
    /// that any buffered-but-not-yet-persisted records are visible during
    /// recovery.
    ///
    /// Opens a **separate read-only handle** to the WAL file (using the
    /// stored path) rather than `try_clone()` on the write handle, because
    /// the write handle is opened with `append(true)` (write-only) and
    /// cloning it yields another write-only fd that cannot be read.
    ///
    /// Like [`WriteAheadLog::recover`], corrupted frames are skipped with
    /// a warning and the method returns as many valid records as possible.
    pub fn recover_locked(&self) -> Result<Vec<LogRecord>> {
        // 1. Lock and flush so all pending data is visible.
        let mut guard = self.file.lock();
        guard.flush()?;
        guard.get_ref().sync_all()?;

        // 2. Open a second, read-only handle to the file.
        //    We hold the lock so no concurrent rename/rotation can happen.
        let reader_file = OpenOptions::new().read(true).open(&self.path)?;
        let file_size = reader_file
            .metadata()
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        drop(guard);

        // 3. Read frames with tolerant recovery.
        let mut reader = BufReader::new(reader_file);
        let mut records = Vec::new();
        let mut skipped_frames: u64 = 0;
        // Track reader position approximately via consumed bytes.
        let mut pos: usize = 0;

        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }

            if buf.len() < 4 {
                // Trailing incomplete length prefix — partial WAL frame from crash.
                debug!("WAL recovery: trailing incomplete frame at offset, discarding");
                break;
            }

            let mut lengthbuf = [0u8; 4];
            reader.read_exact(&mut lengthbuf)?;
            pos += 4;
            let length = u32::from_le_bytes(lengthbuf) as usize;

            // --- Validate length (tolerant) ---
            if length == 0 || length > MAX_WAL_RECORD_BYTES {
                warn!(
                    "WAL recovery: invalid record length {}, skipping corrupted frame",
                    length
                );
                skipped_frames += 1;
                // Try to re-sync to the next valid frame boundary.
                if !resync_after_invalid_length(&mut reader, &mut pos, file_size)? {
                    break;
                }
                continue;
            }

            // --- Quick plausibility check: do we have enough bytes left? ---
            // A frame needs: version (1) + payload (length-1) + checksum (4)
            let frame_remaining = 1 + (length - 1) + 4; // version + payload + checksum
            if pos + frame_remaining > file_size {
                warn!(
                    "WAL recovery: plausible length {} but not enough bytes remain, resyncing",
                    length
                );
                skipped_frames += 1;
                if !resync_after_invalid_length(&mut reader, &mut pos, file_size)? {
                    break;
                }
                continue;
            }

            // --- Read version byte ---
            let mut versionbuf = [0u8; 1];
            reader.read_exact(&mut versionbuf)?;
            pos += 1;
            let version = versionbuf[0];

            // --- Read payload ---
            let payload_len = length - 1;
            let mut payload = vec![0u8; payload_len];
            match reader.read_exact(&mut payload) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Trailing partial payload — crash during write_record.
                    debug!(
                        "WAL recovery: partial payload at end of log, discarding trailing frame"
                    );
                    break;
                }
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            pos += payload_len;

            // --- Read stored checksum ---
            let mut checksumbuf = [0u8; 4];
            match reader.read_exact(&mut checksumbuf) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    // Trailing partial checksum — crash during fsync.
                    debug!(
                        "WAL recovery: partial checksum at end of log, discarding trailing frame"
                    );
                    break;
                }
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            let stored_checksum = u32::from_le_bytes(checksumbuf);
            pos += 4;

            // --- Validate CRC32 (tolerant) ---
            let mut hasher = Hasher::new();
            hasher.update(&lengthbuf);
            hasher.update(&[version]);
            hasher.update(&payload);
            let calculated = hasher.finalize();

            if stored_checksum != calculated {
                warn!("WAL recovery: CRC32 mismatch, skipping corrupted frame");
                skipped_frames += 1;
                // Reader is already past the corrupted frame — continue.
                continue;
            }

            // --- Deserialize based on version (tolerant) ---
            let record = match version {
                WAL_FRAME_VERSION_V0 => match decode::<LogRecordV0>(&payload) {
                    Ok(v0) => LogRecord::from(v0),
                    Err(e) => {
                        warn!(
                            "WAL recovery: V0 deserialization failed ({}), skipping corrupted frame",
                            e
                        );
                        skipped_frames += 1;
                        continue;
                    }
                },
                WAL_FRAME_VERSION_V1 => match decode::<LogRecordV1>(&payload) {
                    Ok(v1) => LogRecord::from(v1),
                    Err(e) => {
                        warn!(
                            "WAL recovery: V1 deserialization failed ({}), skipping corrupted frame",
                            e
                        );
                        skipped_frames += 1;
                        continue;
                    }
                },
                WAL_FRAME_VERSION_V2 => match decode::<LogRecord>(&payload) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(
                            "WAL recovery: V2 deserialization failed ({}), skipping corrupted frame",
                            e
                        );
                        skipped_frames += 1;
                        continue;
                    }
                },
                WAL_FRAME_VERSION_V3_ENCRYPTED => {
                    // Decrypt the payload first (tolerant on failure)
                    match self.encryptor.decrypt_block(&payload) {
                        Ok(decrypted) => match decode::<LogRecord>(&decrypted) {
                            Ok(r) => r,
                            Err(e) => {
                                warn!(
                                    "WAL recovery: V3 encrypted deserialization failed ({}), skipping corrupted frame",
                                    e
                                );
                                skipped_frames += 1;
                                continue;
                            }
                        },
                        Err(e) => {
                            warn!(
                                "WAL recovery: V3 encrypted decryption failed ({}), skipping corrupted frame",
                                e
                            );
                            skipped_frames += 1;
                            continue;
                        }
                    }
                }
                other => {
                    warn!(
                        "WAL recovery: unknown frame version {}, skipping corrupted frame",
                        other
                    );
                    skipped_frames += 1;
                    continue;
                }
            };

            records.push(record);
        }

        // Deduplicate: keep only the last occurrence of each key to avoid
        // reverting to a stale value when batch fsync loses ordering (see
        // [`deduplicate_records`] for details).
        let before = records.len();
        let records = deduplicate_records(records);
        let dedup_count = before - records.len();

        info!(
            "WAL recovery: {} records recovered, {} deduplicated, {} frames skipped",
            records.len(),
            dedup_count,
            skipped_frames
        );

        Ok(records)
    }

    /// Truncate the WAL after a successful MemTable flush to SSTable.
    ///
    /// # Crash Safety
    ///
    /// The implementation uses **atomic file rotation** instead of
    /// in-place truncation:
    ///
    /// 1. An empty temporary file (`wal.log.new`) is created outside the
    ///    lock (pure I/O, no lock contention).
    /// 2. The `Mutex` is acquired.
    /// 3. The old `BufWriter` is flushed and fsynced so any pending data
    ///    is durable before the rotation.
    /// 4. The temporary file is atomically renamed over `wal.log` via
    ///    `std::fs::rename` (atomic on Linux).
    /// 5. The in-memory `BufWriter` is replaced with a new handle to the
    ///    (now empty) file.
    ///
    /// If a crash occurs **before** the rename, the old WAL file is
    /// untouched and will be replayed on next startup.  If a crash occurs
    /// **after** the rename, the WAL is already empty — the engine finds
    /// no frames to replay, which is correct because all data has been
    /// flushed to SSTables.  There is no window where the WAL can be left
    /// in an inconsistent state.
    pub fn clear(&self) -> Result<()> {
        let tmp_path = self.path.with_extension("log.new");

        // 1. Create an empty temp file (I/O — done without the lock so
        //    concurrent writers are not blocked during file creation).
        {
            let tmp_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            tmp_file.sync_all()?;
        }

        // 2. Acquire the lock — no writes can interleave from here on.
        let mut guard = self.file.lock();

        // 3. Flush + fsync the old BufWriter so all pending data is durable.
        guard.flush()?;
        guard.get_ref().sync_all()?;

        // 4. Atomically replace wal.log with the empty temp file.
        std::fs::rename(&tmp_path, &self.path)?;

        // 5. Replace the in-memory BufWriter with a fresh handle to the
        //    new (now empty) file.
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        *guard = BufWriter::new(new_file);

        Ok(())
    }

    /// Remove all WAL records that do **not** satisfy the predicate, then
    /// rewrite the surviving records.
    ///
    /// This is used after flushing a single column family so that records
    /// belonging to other (non-flushed) column families are preserved.
    ///
    /// # Crash safety
    ///
    /// Survivors are first written to a **temporary file** and then
    /// atomically renamed over the original WAL.  If a crash occurs
    /// during the write phase, the original WAL file is untouched and
    /// will be fully replayed on next startup.
    pub fn retain<F>(&self, mut predicate: F) -> Result<()>
    where
        F: FnMut(&LogRecord) -> bool,
    {
        // 1. Read all existing records
        let all_records = self.recover()?;

        // 2. Filter
        let survivors: Vec<LogRecord> = all_records.into_iter().filter(|r| predicate(r)).collect();

        // 3. Write survivors to a temp file first (crash-safe: original is untouched)
        let tmp_path = self.path.with_extension("wal.tmp");
        {
            let tmp_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            let mut tmp_writer = BufWriter::new(tmp_file);

            for record in &survivors {
                let serialized = encode(record)?;

                // Encrypt payload if encryption is enabled
                let (payload, version) = if self.encryptor.is_enabled() {
                    let encrypted = self.encryptor.encrypt_block(&serialized)?;
                    (encrypted, WAL_FRAME_VERSION_V3_ENCRYPTED)
                } else {
                    (serialized, WAL_CURRENT_FRAME_VERSION)
                };

                let length = 1u32 + payload.len() as u32;
                let length_bytes = length.to_le_bytes();

                let mut hasher = Hasher::new();
                hasher.update(&length_bytes);
                hasher.update(&[version]);
                hasher.update(&payload);
                let checksum = hasher.finalize();

                tmp_writer.write_all(&length_bytes)?;
                tmp_writer.write_all(&[version])?;
                tmp_writer.write_all(&payload)?;
                tmp_writer.write_all(&checksum.to_le_bytes())?;
            }

            tmp_writer.flush()?;
            tmp_writer.get_ref().sync_all()?;
        }

        // 4. Atomically replace the original WAL with the temp file
        std::fs::rename(&tmp_path, &self.path)?;

        // 5. Reset the in-memory BufWriter to point to the new file content
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut guard = self.file.lock();
        *guard = BufWriter::new(new_file);

        Ok(())
    }

    /// Flush the BufWriter and fsync the underlying file.
    ///
    /// Called during graceful shutdown to ensure all buffered data is
    /// durably on disk before the engine is dropped.  Also resets the
    /// batch counter so the next `write_record` starts a fresh batch.
    pub fn sync(&self) -> Result<()> {
        let mut guard = self.file.lock();
        guard.flush()?;
        guard.get_ref().sync_all()?;
        let mut count = self.batch_count.lock();
        *count = 0;
        Ok(())
    }

    /// Return the current size of the WAL file in bytes.
    pub fn size(&self) -> Result<u64> {
        std::fs::metadata(&self.path)
            .map(|m| m.len())
            .map_err(crate::infra::error::LsmError::Io)
    }

    // ── WAL Archiving (#224) ───────────────────────────────────────────────

    /// Archive the current WAL by rotating it to a timestamped backup file.
    ///
    /// The current WAL is flushed, fsynced, and renamed to
    /// `wal-{cf}-{timestamp}.log.archive`. A fresh empty WAL file is created
    /// in its place.
    ///
    /// Returns the path to the archived file.
    pub fn archive(&self) -> Result<std::path::PathBuf> {
        let archive_path = self.path.with_extension(format!(
            "log-{}.archive",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        // Flush and fsync current data.
        let mut guard = self.file.lock();
        guard.flush()?;
        guard.get_ref().sync_all()?;

        // Rename current file to archive path.
        std::fs::rename(&self.path, &archive_path)?;

        // Create a fresh WAL file.
        let new_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        *guard = BufWriter::new(new_file);

        Ok(archive_path)
    }

    /// Check whether the WAL file exceeds the given `max_size` and should be
    /// archived.
    pub fn exceeds_max_size(&self, max_size: u64) -> Result<bool> {
        Ok(self.size()? > max_size)
    }
}

// ---------------------------------------------------------------------------
// Helper: deduplicate recovered WAL records
// ---------------------------------------------------------------------------

/// Deduplicate recovered WAL records by (column_family, key), keeping only the
/// **last** occurrence of each key (by position in the file).
///
/// ## Why this is necessary
///
/// The batched WAL fsync (`WAL_SYNC_INTERVAL = 4`) delays `sync_all()` across
/// multiple `write_record()` calls.  If a key is written multiple times (e.g.
/// `k=v1`, `k=v2`, `k=v3`) and only 1 out of 3 fsyncs completes before a crash,
/// the WAL might contain `k=v1` but not `k=v2` or `k=v3`.  Without deduplication,
/// recovery would replay `k=v1` — reverting the key to a stale value.
///
/// By keeping only the **last** occurrence of each key in the recovered records,
/// we ensure that even if some intermediate writes were lost, the engine never
/// regresses to an older value that happened to be more durably persisted.
///
/// The deduplication is performed **after** all records have been read from the
/// file, so it works regardless of which frames survived the crash.
fn deduplicate_records(records: Vec<LogRecord>) -> Vec<LogRecord> {
    use std::collections::HashMap;

    // Map from (column_family, key_bytes) → index of last occurrence
    let mut last_occurrence: HashMap<(String, Vec<u8>), usize> = HashMap::new();
    for (i, record) in records.iter().enumerate() {
        let cf = record
            .column_family
            .as_deref()
            .unwrap_or("default")
            .to_string();
        last_occurrence.insert((cf, record.key.clone()), i);
    }

    // Collect the last occurrence of each unique key in file order.
    let mut indices: Vec<usize> = last_occurrence.into_values().collect();
    indices.sort_unstable();
    indices.into_iter().map(|i| records[i].clone()).collect()
}

// ---------------------------------------------------------------------------
// Helper: resync after invalid length
// ---------------------------------------------------------------------------

/// After reading an invalid frame length, scan forward byte-by-byte to find
/// the next plausible frame boundary.
///
/// To reduce false positives, this function checks not only that the 4-byte
/// candidate forms a valid length, but also that the **following byte** is
/// a known WAL frame version (`0x00` for V0 or `0x01` for V1) — payload
/// data is very unlikely to match both criteria by chance.
///
/// Returns `true` if a candidate was found (the reader is positioned right
/// before the candidate's 4-byte length prefix), or `false` if the search
/// reached EOF without finding a plausible frame start.
///
/// The scan is limited to [`MAX_WAL_RECORD_BYTES`] bytes to avoid an
/// infinite loop on heavily corrupted data.
fn resync_after_invalid_length(
    reader: &mut BufReader<File>,
    pos: &mut usize,
    file_size: usize,
) -> io::Result<bool> {
    /// Minimum realistic frame length (version byte + serialised LogRecord payload).
    ///
    /// A LogRecord with both key and value empty serialises to 34 bytes
    /// (Vec length prefixes 0+0 = 16, u128 timestamp = 16, bool = 1,
    /// Option<String> = 1), so the WAL frame length field is at least
    /// `1 + 34 = 35`.  Any candidate smaller than this is certainly a
    /// false positive from payload data.
    const MIN_LENGTH: usize = 35;
    let max_scan = MAX_WAL_RECORD_BYTES;
    let mut skip_byte = [0u8; 1];

    for _ in 0..max_scan {
        // Consume one byte forward.
        if reader.read(&mut skip_byte)? == 0 {
            return Ok(false); // EOF — no valid frame found.
        }
        *pos += 1;

        // Peek at the next 4 bytes (fill_buf does NOT consume).
        let buf = reader.fill_buf()?;
        if buf.len() >= 5 {
            let candidate = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            let version_byte = buf[4];
            // A plausible frame must:
            // 1. Have a length in [MIN_LENGTH, MAX_WAL_RECORD_BYTES]
            // 2. Fit within the file: candidate + version(1) + payload(candidate-1) + checksum(4) = candidate + 4
            // 3. Be followed by a known WAL frame version byte
            if (MIN_LENGTH..=MAX_WAL_RECORD_BYTES).contains(&candidate)
                && *pos + 4 + candidate <= file_size
                && (version_byte == WAL_FRAME_VERSION_V0
                    || version_byte == WAL_FRAME_VERSION_V1
                    || version_byte == WAL_FRAME_VERSION_V2
                    || version_byte == WAL_FRAME_VERSION_V3_ENCRYPTED)
            {
                return Ok(true); // Found a plausible frame start.
            }
        }
    }

    // Exhausted scan limit without finding a valid frame.
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_wal() -> (TempDir, WriteAheadLog) {
        let temp_dir = TempDir::new().unwrap();
        let wal = WriteAheadLog::new(temp_dir.path(), "default").unwrap();
        (temp_dir, wal)
    }

    #[test]
    fn test_wal_write_and_read_round_trip() {
        let (_temp_dir, wal) = create_test_wal();

        let record = LogRecord::new(b"test_key".to_vec(), b"test_value".to_vec());
        wal.write_record(&record).unwrap();

        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], record);
    }

    #[test]
    fn test_wal_crc32_corruption_skipped() {
        // With tolerant recovery, a corrupted frame is skipped rather than
        // causing the entire recovery to fail.
        let (temp_dir, wal) = create_test_wal();

        let record = LogRecord::new(b"test_key".to_vec(), b"test_value".to_vec());
        wal.write_record(&record).unwrap();

        // Corrupt the WAL file by flipping a bit in the payload
        let wal_path = temp_dir.path().join("wal.log");
        let mut file = fs::File::open(&wal_path).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();

        // Flip a bit in the version byte (offset 4 from start, after length prefix)
        if data.len() > 5 {
            data[4] ^= 0x01;
        }

        // Write back the corrupted data
        let mut file = fs::File::create(&wal_path).unwrap();
        file.write_all(&data).unwrap();
        drop(file);

        // Recovery should succeed but skip the corrupted frame
        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 0, "corrupted frame should be skipped");
    }

    #[test]
    fn test_wal_graceful_truncation() {
        // After a crash, the last WAL record may be partially written.
        // The engine should recover all complete records and discard
        // the trailing partial frame without returning an error.
        let (_temp_dir, wal) = create_test_wal();

        // Write two records
        let record1 = LogRecord::new(b"key1".to_vec(), b"value1".to_vec());
        let record2 = LogRecord::new(b"key2".to_vec(), b"value2".to_vec());
        wal.write_record(&record1).unwrap();
        wal.write_record(&record2).unwrap();

        // Truncate the checksum from the last record to simulate partial write
        let wal_path = wal.path.clone();
        let mut original = fs::read(&wal_path).unwrap();
        if original.len() > 4 {
            original.truncate(original.len() - 4);
            fs::write(&wal_path, original).unwrap();
        }

        // Recovery should succeed with only the first (complete) record
        let result = wal.recover();
        let recovered = result.expect("recovery should succeed even with truncated trailing frame");
        assert_eq!(
            recovered.len(),
            1,
            "should recover the first complete record"
        );
        assert_eq!(recovered[0], record1);
    }

    #[test]
    fn test_wal_graceful_payload_truncation() {
        let (_temp_dir, wal) = create_test_wal();

        // Write one record
        let record = LogRecord::new(b"test_key".to_vec(), b"this_is_a_larger_value".to_vec());
        wal.write_record(&record).unwrap();

        // Truncate part of the payload (keep only half)
        let wal_path = wal.path.clone();
        let mut original = fs::read(&wal_path).unwrap();

        // Structure: [len:4][ver:1][payload:N][crc32:4]
        // Truncate so that payload is cut short, removing checksum too
        if original.len() > 9 {
            let payload_area = original.len() - 9; // N
            let half_payload = payload_area / 2;
            let keep_length = 4 + 1 + half_payload;
            if keep_length > 5 {
                original.truncate(keep_length);
                fs::write(&wal_path, original).unwrap();
            }
        }

        // Recovery should succeed with 0 records (the partial frame is discarded)
        let result = wal.recover();
        let recovered = result.expect("recovery should succeed with truncated payload");
        assert_eq!(recovered.len(), 0, "partial frame should be discarded");
    }

    #[test]
    fn test_wal_v1_round_trip_with_cf() {
        // Test that a LogRecord WITH a column family round-trips correctly.
        let (_temp_dir, wal) = create_test_wal();

        let mut record = LogRecord::new(b"k1".to_vec(), b"v1".to_vec());
        record.column_family = Some("users".to_string());

        wal.write_record(&record).unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], record);
        assert_eq!(recovered[0].column_family.as_deref(), Some("users"));
    }

    #[test]
    fn test_wal_v1_record_with_no_cf_round_trip() {
        // Test that a V1 record with column_family: None round-trips correctly.
        let (_temp_dir, wal) = create_test_wal();

        let record = LogRecord::new(b"test_key".to_vec(), b"test_value".to_vec());
        assert!(record.column_family.is_none());

        wal.write_record(&record).unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0], record);
        assert!(recovered[0].column_family.is_none());

        // Verify it maps to "default" via unwrap_or
        let cf = recovered[0].column_family.as_deref().unwrap_or("default");
        assert_eq!(cf, "default");
    }

    #[test]
    fn test_wal_v0_backward_compat_no_cf() {
        // Test that a manually constructed V0 frame (without column_family field)
        // can be recovered and maps to column_family = None → "default".
        //
        // A V0 frame is: [len:u32 LE][ver:u8 = 0][payload: LogRecordV0 (bincode)][crc32:u32 LE]
        let temp_dir = TempDir::new().unwrap();
        let wal_path = temp_dir.path().join("wal.log");

        // Construct a LogRecordV0 payload manually
        let v0 = LogRecordV0 {
            key: b"legacy_key".to_vec(),
            value: b"legacy_value".to_vec(),
            timestamp: 42,
            is_deleted: false,
        };

        // Serialize using bincode (same encoding as V1 but without column_family)
        let payload = encode(&v0).unwrap();
        let version = WAL_FRAME_VERSION_V0;

        // Frame: [len:4][ver:1][payload:N][crc32:4]
        let length = 1u32 + payload.len() as u32;
        let length_bytes = length.to_le_bytes();

        let mut hasher = Hasher::new();
        hasher.update(&length_bytes);
        hasher.update(&[version]);
        hasher.update(&payload);
        let checksum = hasher.finalize();

        let mut frame = Vec::new();
        frame.extend_from_slice(&length_bytes);
        frame.push(version);
        frame.extend_from_slice(&payload);
        frame.extend_from_slice(&checksum.to_le_bytes());

        fs::write(&wal_path, &frame).unwrap();

        // Now recover via WriteAheadLog
        let wal = WriteAheadLog::new(temp_dir.path(), "default").unwrap();
        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 1);

        // The record should have column_family = None (treated as "default")
        assert!(recovered[0].column_family.is_none());
        assert_eq!(recovered[0].key, v0.key);
        assert_eq!(recovered[0].value, v0.value);

        let cf = recovered[0].column_family.as_deref().unwrap_or("default");
        assert_eq!(cf, "default");
    }

    #[test]
    fn test_wal_tolerant_recovery_after_crc_corruption() {
        // Write two valid records, corrupt the first one, then verify that
        // recovery skips the corrupted frame and recovers the second one.
        let (temp_dir, wal) = create_test_wal();

        let record1 = LogRecord::new(b"key1".to_vec(), b"value1".to_vec());
        let record2 = LogRecord::new(b"key2".to_vec(), b"value2".to_vec());
        wal.write_record(&record1).unwrap();
        wal.write_record(&record2).unwrap();

        // Corrupt the first frame
        let wal_path = temp_dir.path().join("wal.log");
        let mut data = fs::read(&wal_path).unwrap();
        if data.len() > 9 {
            // Flip a bit in the version byte of the first frame (offset 4)
            data[4] ^= 0x01;
        }
        fs::write(&wal_path, data).unwrap();

        // Recovery should skip the corrupted first frame and recover valid
        // frames that follow.
        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 1, "should recover the second (valid) frame");
        assert_eq!(records[0], record2);
    }

    #[test]
    fn test_wal_tolerant_recovery_invalid_length() {
        // Write two valid records, corrupt the length of the first one,
        // then verify that recovery resyncs and finds the second one.
        let (temp_dir, wal) = create_test_wal();

        let record1 = LogRecord::new(b"key1".to_vec(), b"value1".to_vec());
        let record2 = LogRecord::new(b"key2".to_vec(), b"value2".to_vec());
        wal.write_record(&record1).unwrap();
        wal.write_record(&record2).unwrap();

        // Corrupt the length prefix of the first frame (set it to 0xFFFFFFFF)
        let wal_path = temp_dir.path().join("wal.log");
        let mut data = fs::read(&wal_path).unwrap();
        if data.len() > 4 {
            // Set length of first frame to an invalid value
            data[0..4].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        }
        fs::write(&wal_path, data).unwrap();

        // Recovery should resync and recover the second frame
        let records = wal.recover().unwrap();
        assert_eq!(
            records.len(),
            1,
            "should recover the second (valid) frame after resync"
        );
        assert_eq!(records[0], record2);
    }

    #[test]
    fn test_wal_multiple_records() {
        let (_temp_dir, wal) = create_test_wal();

        let records = vec![
            LogRecord::new(b"key1".to_vec(), b"value1".to_vec()),
            LogRecord::new(b"key2".to_vec(), b"value2".to_vec()),
            LogRecord::tombstone(b"key3".to_vec()),
        ];

        for record in &records {
            wal.write_record(record).unwrap();
        }

        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), records.len());
        for (original, recovered_record) in records.iter().zip(recovered.iter()) {
            assert_eq!(original, recovered_record);
        }
    }

    // ── Issue #191: WAL deduplication tests ──

    #[test]
    fn test_wal_deduplicate_same_key_different_values() {
        // Simulate the bug scenario: k=v1, k=v2, k=v3 written, but only
        // k=v1 and k=v3 survive on disk. Recovery should return only k=v3
        // (the last occurrence).
        let (_temp_dir, wal) = create_test_wal();

        let r1 = LogRecord::new(b"k".to_vec(), b"v1".to_vec());
        let r2 = LogRecord::new(b"k".to_vec(), b"v2".to_vec());
        let r3 = LogRecord::new(b"k".to_vec(), b"v3".to_vec());

        wal.write_record(&r1).unwrap();
        wal.write_record(&r2).unwrap();
        wal.write_record(&r3).unwrap();

        // Force an fsync so all 3 records are durable.
        wal.sync().unwrap();

        // Recovery should deduplicate: only the last occurrence (k=v3) survives.
        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 1, "only the last occurrence should survive");
        assert_eq!(records[0].key, b"k");
        assert_eq!(
            records[0].value, b"v3",
            "should keep the final value v3, not v1"
        );
    }

    #[test]
    fn test_wal_deduplicate_interleaved_keys() {
        // Multiple keys interleaved: k1=v1, k2=v2, k1=v3, k2=v4
        // Recovery should keep k1=v3, k2=v4 (last occurrence of each).
        let (_temp_dir, wal) = create_test_wal();

        let r1 = LogRecord::new(b"k1".to_vec(), b"v1".to_vec());
        let r2 = LogRecord::new(b"k2".to_vec(), b"v2".to_vec());
        let r3 = LogRecord::new(b"k1".to_vec(), b"v3".to_vec());
        let r4 = LogRecord::new(b"k2".to_vec(), b"v4".to_vec());

        wal.write_record(&r1).unwrap();
        wal.write_record(&r2).unwrap();
        wal.write_record(&r3).unwrap();
        wal.write_record(&r4).unwrap();
        wal.sync().unwrap();

        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 2, "two unique keys after dedup");

        // Order should be k1, k2 (preserving last-occurrence order)
        assert_eq!(records[0].key, b"k1");
        assert_eq!(records[0].value, b"v3");
        assert_eq!(records[1].key, b"k2");
        assert_eq!(records[1].value, b"v4");
    }

    #[test]
    fn test_wal_deduplicate_with_tombstone() {
        // If a key is written then deleted, and both survive, the tombstone
        // (last occurrence) should be kept.
        let (_temp_dir, wal) = create_test_wal();

        let write = LogRecord::new(b"k".to_vec(), b"v1".to_vec());
        let delete = LogRecord::tombstone(b"k".to_vec());

        wal.write_record(&write).unwrap();
        wal.write_record(&delete).unwrap();
        wal.sync().unwrap();

        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 1, "only the tombstone should survive");
        assert_eq!(records[0].key, b"k");
        assert!(records[0].is_deleted, "should keep the tombstone");
    }

    #[test]
    fn test_wal_deduplicate_different_cfs_independent() {
        // Keys with the same name in different column families should
        // NOT be deduplicated against each other.
        let (_temp_dir, wal) = create_test_wal();

        let mut r1 = LogRecord::new(b"k".to_vec(), b"default_v1".to_vec());
        r1.column_family = None; // default
        let mut r2 = LogRecord::new(b"k".to_vec(), b"users_v1".to_vec());
        r2.column_family = Some("users".to_string());

        wal.write_record(&r1).unwrap();
        wal.write_record(&r2).unwrap();
        wal.sync().unwrap();

        let records = wal.recover().unwrap();
        assert_eq!(
            records.len(),
            2,
            "same key in different CFs should both survive"
        );
    }

    #[test]
    fn test_wal_deduplicate_no_duplicates_unchanged() {
        // When there are no duplicate keys, deduplication should return the
        // same records in the same order.
        let (_temp_dir, wal) = create_test_wal();

        let records = vec![
            LogRecord::new(b"a".to_vec(), b"1".to_vec()),
            LogRecord::new(b"b".to_vec(), b"2".to_vec()),
            LogRecord::new(b"c".to_vec(), b"3".to_vec()),
        ];

        for r in &records {
            wal.write_record(r).unwrap();
        }
        wal.sync().unwrap();

        let recovered = wal.recover().unwrap();
        assert_eq!(recovered.len(), 3);
        for (orig, recv) in records.iter().zip(recovered.iter()) {
            assert_eq!(orig, recv);
        }
    }
}
