use crate::core::log_record::LogRecord;
use crate::infra::codec::{decode, encode};
use crate::infra::error::{LsmError, Result};
use crc32fast::Hasher;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tracing::debug;

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
/// `[length: u32 LE][payload: bytes][crc32: u32 LE]`
///
/// The CRC32 checksum is calculated over the payload and provides protection
/// against partial writes, bit rot, and other forms of data corruption.
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
    /// The on-disk format is a length-prefixed frame with CRC32 checksum:
    /// `[length: u32 LE][payload: bytes][crc32: u32 LE]`
    pub fn write_record(&self, record: &LogRecord) -> Result<()> {
        let serialized = encode(record)?;
        let length = serialized.len() as u32;

        // Calculate CRC32 over the payload
        let mut hasher = Hasher::new();
        hasher.update(&serialized);
        let checksum = hasher.finalize();

        let mut writer = self.file.lock();

        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(&serialized)?;
        writer.write_all(&checksum.to_le_bytes())?;
        writer.flush()?;
        writer.get_ref().sync_all()?;

        debug!("WAL persisted: key={}, ts={}", record.key, record.timestamp);
        Ok(())
    }

    /// Replay all records persisted in the WAL.
    ///
    /// Called once during engine initialisation.  Returns an error if the
    /// file contains a truncated or malformed frame, indicating a partial
    /// write that was not fsynced before a crash.
    ///
    /// The expected on-disk format is:
    /// `[length: u32 LE][payload: bytes][crc32: u32 LE]`
    ///
    /// # Compatibility Note
    ///
    /// WALs created by versions prior to this change (without CRC32) will
    /// be rejected with a `CorruptedData` error. The engine requires all
    /// WAL frames to include the checksum for crash recovery safety.
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
                return Err(LsmError::WalCorruption);
            }

            let mut lengthbuf = [0u8; 4];
            reader.read_exact(&mut lengthbuf)?;
            let length = u32::from_le_bytes(lengthbuf) as usize;

            if length == 0 || length > MAX_WAL_RECORD_BYTES {
                return Err(LsmError::CorruptedData(
                    "Invalid WAL record length".to_string(),
                ));
            }

            let mut payload = vec![0u8; length];
            if let Err(e) = reader.read_exact(&mut payload) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    return Err(LsmError::CorruptedData(
                        "WAL record payload truncated".to_string(),
                    ));
                }
                return Err(e.into());
            }

            // Read stored checksum
            let mut checksumbuf = [0u8; 4];
            if let Err(e) = reader.read_exact(&mut checksumbuf) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    return Err(LsmError::CorruptedData(
                        "WAL record checksum truncated".to_string(),
                    ));
                }
                return Err(e.into());
            }
            let stored_checksum = u32::from_le_bytes(checksumbuf);

            // Recalculate and validate checksum
            let mut hasher = Hasher::new();
            hasher.update(&payload);
            let calculated = hasher.finalize();

            if stored_checksum != calculated {
                return Err(LsmError::CorruptedData(
                    "WAL record CRC32 mismatch: log may be truncated or corrupted".to_string(),
                ));
            }

            let record: LogRecord = decode(&payload).map_err(|_| {
                LsmError::CorruptedData("WAL record deserialization failed".to_string())
            })?;
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
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.file.lock();

        // 1. Flush the BufWriter's in-process buffer to the OS page cache.
        guard.flush()?;

        // 2-4. Operate on the underlying File directly.
        //      get_mut() gives us &mut File without releasing the BufWriter.
        {
            let file = guard.get_mut();
            file.sync_all()?; // 2. fsync — durable before we erase
            file.set_len(0)?; // 3. truncate in-place
            file.seek(SeekFrom::Start(0))?; // 4. reset write position
        }

        // The BufWriter's internal state (position counter) is now stale.
        // Recreate it around the same file descriptor to reset the counter.
        // We must move the File out of the old BufWriter to do so.
        //
        // SAFETY: guard still holds the Mutex; no other thread can observe
        // the intermediate state.
        let file = guard.get_mut().try_clone()?;
        *guard = BufWriter::new(file);

        Ok(())
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

        let record = LogRecord::new("test_key".to_string(), b"test_value".to_vec());
        wal.write_record(&record).unwrap();

        let records = wal.recover().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0], record);
    }

    #[test]
    fn test_wal_crc32_corruption_detection() {
        let (temp_dir, wal) = create_test_wal();

        let record = LogRecord::new("test_key".to_string(), b"test_value".to_vec());
        wal.write_record(&record).unwrap();

        // Corrupt the WAL file by flipping a bit in the payload
        let wal_path = temp_dir.path().join("wal.log");
        let mut file = fs::File::open(&wal_path).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();

        // Flip a bit in the first byte of the payload (after the length prefix)
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
    fn test_wal_crc32_truncation_detection() {
        let (temp_dir, wal) = create_test_wal();

        // Write a record first
        let record = LogRecord::new("test_key".to_string(), b"test_value".to_vec());
        wal.write_record(&record).unwrap();

        // Truncate the checksum from the WAL file
        let wal_path = temp_dir.path().join("wal.log");
        let mut original = fs::read(&wal_path).unwrap();
        if original.len() > 4 {
            original.truncate(original.len() - 4);
            fs::write(&wal_path, original).unwrap();
        }

        // Recovery should fail with truncated checksum
        let result = wal.recover();
        assert!(result.is_err());
        match result.unwrap_err() {
            LsmError::CorruptedData(msg) => {
                assert!(
                    msg.contains("checksum") || msg.contains("CRC32") || msg.contains("truncated")
                );
            }
            _ => panic!("Expected CorruptedData error"),
        }
    }

    #[test]
    fn test_wal_multiple_records() {
        let (_temp_dir, wal) = create_test_wal();

        let records = vec![
            LogRecord::new("key1".to_string(), b"value1".to_vec()),
            LogRecord::new("key2".to_string(), b"value2".to_vec()),
            LogRecord::tombstone("key3".to_string()),
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
