use crate::core::log_record::LogRecord;
use crate::infra::codec::{decode, encode};
use crate::infra::error::{LsmError, Result};
use crc32fast::Hasher;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tracing::debug;

/// WAL frame version constants for backward compatibility.
///
/// - Version 0: LogRecord serialized WITHOUT `column_family` (original format).
/// - Version 1: LogRecord serialized WITH `column_family`.
pub(crate) const WAL_FRAME_VERSION_V0: u8 = 0;
pub(crate) const WAL_FRAME_VERSION_V1: u8 = 1;
pub(crate) const WAL_CURRENT_FRAME_VERSION: u8 = WAL_FRAME_VERSION_V1;

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
}

const MAX_WAL_RECORD_BYTES: usize = 32 * 1024 * 1024; // 32 MiB

impl WriteAheadLog {
    pub fn new(dir_path: &std::path::Path) -> Result<Self> {
        let wal_path = dir_path.join("wal.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&wal_path)?;

        Ok(Self {
            file: Mutex::new(BufWriter::new(file)),
            path: wal_path,
        })
    }

    /// Append a single record to the WAL and fsync.
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
        let version = WAL_CURRENT_FRAME_VERSION;

        // `length` includes version byte + payload bytes
        let length = 1u32 + serialized.len() as u32;

        // Calculate CRC32 over (length + version + payload)
        let length_bytes = length.to_le_bytes();
        let mut hasher = Hasher::new();
        hasher.update(&length_bytes);
        hasher.update(&[version]);
        hasher.update(&serialized);
        let checksum = hasher.finalize();

        let mut writer = self.file.lock();

        writer.write_all(&length_bytes)?;
        writer.write_all(&[version])?;
        writer.write_all(&serialized)?;
        writer.write_all(&checksum.to_le_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;

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
            let version = WAL_CURRENT_FRAME_VERSION;
            let length = 1u32 + serialized.len() as u32;
            let length_bytes = length.to_le_bytes();

            let mut hasher = Hasher::new();
            hasher.update(&length_bytes);
            hasher.update(&[version]);
            hasher.update(&serialized);
            let checksum = hasher.finalize();

            let mut frame = Vec::with_capacity(4 + 1 + serialized.len() + 4);
            frame.extend_from_slice(&length_bytes);
            frame.push(version);
            frame.extend_from_slice(&serialized);
            frame.extend_from_slice(&checksum.to_le_bytes());
            frames.push(frame);
        }

        let mut writer = self.file.lock();
        for frame in &frames {
            writer.write_all(frame)?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;

        debug!("WAL batch persisted: {} records", records.len());
        Ok(())
    }

    /// Replay all records persisted in the WAL.
    ///
    /// Called once during engine initialisation.  Returns an error if the
    /// file contains a truncated or malformed frame, indicating a partial
    /// write that was not fsynced before a crash.
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
    /// WALs created by versions prior to the CRC32 addition will be rejected
    /// with a `CorruptedData` error. The engine requires all WAL frames to
    /// include the checksum for crash recovery safety.
    pub fn recover(&self) -> Result<Vec<LogRecord>> {
        let mut records = Vec::new();
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);

        loop {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }

            if buf.len() < 4 {
                // Trailing incomplete length prefix — partial WAL frame from crash
                debug!(
                    "WAL recovery: trailing incomplete frame at offset {}, discarding",
                    buf.len()
                );
                break;
            }

            let mut lengthbuf = [0u8; 4];
            reader.read_exact(&mut lengthbuf)?;
            let length = u32::from_le_bytes(lengthbuf) as usize;

            if length == 0 || length > MAX_WAL_RECORD_BYTES {
                return Err(LsmError::CorruptedData(
                    "Invalid WAL record length".to_string(),
                ));
            }

            if length < 1 {
                return Err(LsmError::CorruptedData(
                    "WAL record too short (missing version byte)".to_string(),
                ));
            }

            // Read version byte
            let mut versionbuf = [0u8; 1];
            reader.read_exact(&mut versionbuf)?;
            let version = versionbuf[0];

            // The payload is length - 1 (excluding the version byte itself)
            let payload_len = length - 1;
            let mut payload = vec![0u8; payload_len];
            if let Err(e) = reader.read_exact(&mut payload) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    // Trailing partial payload — crash during write_record
                    debug!(
                        "WAL recovery: partial payload at end of log, discarding trailing frame"
                    );
                    break;
                }
                return Err(e.into());
            }

            // Read stored checksum
            let mut checksumbuf = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut checksumbuf) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    // Trailing partial checksum — crash during write_record fsync
                    debug!(
                        "WAL recovery: partial checksum at end of log, discarding trailing frame"
                    );
                    break;
                }
                return Err(e.into());
            }
            let stored_checksum = u32::from_le_bytes(checksumbuf);

            // Recalculate and validate checksum over (length + version + payload)
            let mut hasher = Hasher::new();
            hasher.update(&lengthbuf);
            hasher.update(&[version]);
            hasher.update(&payload);
            let calculated = hasher.finalize();

            if stored_checksum != calculated {
                return Err(LsmError::CorruptedData(
                    "WAL record CRC32 mismatch: log may be truncated or corrupted".to_string(),
                ));
            }

            // Deserialize based on version
            let record = match version {
                WAL_FRAME_VERSION_V0 => {
                    let v0: LogRecordV0 = decode(&payload).map_err(|_| {
                        LsmError::CorruptedData("WAL record V0 deserialization failed".to_string())
                    })?;
                    LogRecord::from(v0)
                }
                WAL_FRAME_VERSION_V1 => {
                    let r: LogRecord = decode(&payload).map_err(|_| {
                        LsmError::CorruptedData("WAL record V1 deserialization failed".to_string())
                    })?;
                    r
                }
                other => {
                    return Err(LsmError::CorruptedData(format!(
                        "Unknown WAL frame version: {}",
                        other
                    )));
                }
            };

            records.push(record);
        }

        Ok(records)
    }

    /// Truncate the WAL after a successful MemTable flush to SSTable.
    ///
    /// # Crash Safety
    ///
    /// All mutations happen on the **single file descriptor** already held
    /// inside the `BufWriter`, while the `Mutex` is held.  There is no
    /// second `open()` call and therefore no window between a truncate and
    /// a reopen where a crash could leave the WAL in an inconsistent state.
    ///
    /// Execution order under the lock:
    ///
    /// 1. `flush()`    — drain the `BufWriter` user-space buffer to the OS
    /// 2. `sync_all()` — fsync: ensure all bytes are on durable storage
    /// 3. `set_len(0)` — atomically truncate the file to zero bytes
    /// 4. `seek(0)`    — reset the write cursor to offset 0
    ///
    /// # Implementation note
    ///
    /// After truncation and seek the `BufWriter`'s own buffer is empty
    /// (flushed in step 1), so the next write will correctly start at
    /// offset 0 without needing to recreate the `BufWriter`.  We no
    /// longer use `try_clone()` (which can fail on some platforms) to
    /// create a new file handle.
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.file.lock();

        // 1. Flush the BufWriter's in-process buffer to the OS page cache.
        guard.flush()?;

        // 2-4. Operate on the underlying File directly.
        //      get_mut() gives us &mut File without releasing the BufWriter.
        let file = guard.get_mut();
        file.sync_all()?; // 2. fsync — durable before we erase
        file.set_len(0)?; // 3. truncate in-place
        file.seek(SeekFrom::Start(0))?; // 4. reset write position

        // BufWriter was flushed in step 1 — its internal buffer is empty.
        // The underlying File now has length 0 and position 0.
        // No need to recreate the BufWriter; the next write is correct.
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
                let version = WAL_CURRENT_FRAME_VERSION;
                let length = 1u32 + serialized.len() as u32;
                let length_bytes = length.to_le_bytes();

                let mut hasher = Hasher::new();
                hasher.update(&length_bytes);
                hasher.update(&[version]);
                hasher.update(&serialized);
                let checksum = hasher.finalize();

                tmp_writer.write_all(&length_bytes)?;
                tmp_writer.write_all(&[version])?;
                tmp_writer.write_all(&serialized)?;
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

    /// Return the current size of the WAL file in bytes.
    pub fn size(&self) -> Result<u64> {
        std::fs::metadata(&self.path)
            .map(|m| m.len())
            .map_err(crate::infra::error::LsmError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_wal() -> (TempDir, WriteAheadLog) {
        let temp_dir = TempDir::new().unwrap();
        let wal = WriteAheadLog::new(temp_dir.path()).unwrap();
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
    fn test_wal_crc32_corruption_detection() {
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

        // Recovery should fail with CRC32 mismatch
        let result = wal.recover();
        assert!(result.is_err());
        match result.unwrap_err() {
            LsmError::CorruptedData(msg) => {
                assert!(msg.contains("CRC32 mismatch"));
            }
            _ => panic!("Expected CorruptedData error"),
        }
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
        let wal = WriteAheadLog::new(temp_dir.path()).unwrap();
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
}
