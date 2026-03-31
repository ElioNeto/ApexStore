use crate::core::log_record::LogRecord;
use crate::infra::codec::{decode, encode};
use crate::infra::error::{LsmError, Result};
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
    /// The on-disk format is a length-prefixed frame:
    /// `[length: u32 LE][payload: bytes]`
    pub fn write_record(&self, record: &LogRecord) -> Result<()> {
        let serialized = encode(record)?;
        let length = serialized.len() as u32;

        let mut writer = self.file.lock();

        writer.write_all(&length.to_le_bytes())?;
        writer.write_all(&serialized)?;
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
                return Err(LsmError::WalCorruption);
            }

            let mut buffer = vec![0u8; length];
            if let Err(e) = reader.read_exact(&mut buffer) {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    return Err(LsmError::WalCorruption);
                }
                return Err(e.into());
            }

            let record: LogRecord = decode(&buffer).map_err(|_| LsmError::WalCorruption)?;
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
    ///   1. `flush()`    — drain the `BufWriter` user-space buffer to the OS
    ///   2. `sync_all()` — fsync: ensure all bytes are on durable storage
    ///   3. `set_len(0)` — atomically truncate the file to zero bytes
    ///   4. `seek(0)`    — reset the write cursor so the next record lands
    ///                     at offset 0
    pub fn clear(&self) -> Result<()> {
        let mut guard = self.file.lock();

        // 1. Flush the BufWriter's in-process buffer to the OS page cache.
        guard.flush()?;

        // 2–4. Operate on the underlying File directly.
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
