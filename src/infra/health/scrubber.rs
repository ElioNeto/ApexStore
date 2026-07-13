//! Data integrity scrubber.
//!
//! A background thread that periodically reads all SSTable files and verifies
//! their CRC32 checksums to detect silent data corruption (bit rot). Results
//! are collected and can be queried via the [`results`](DataScrubber::results)
//! method.
//!
//! This module also provides file-level scrubbing via [`scrub_file`] and
//! engine-integrated orphan detection via [`scrub_with_version_set`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::core::engine::Engine;
use crate::storage::builder::MetaBlock;
use crate::storage::cache::Cache;

/// Outcome of a single scrub operation on one SSTable file.
#[derive(Debug, Clone)]
pub struct ScrubResult {
    /// Path to the scrubbed file.
    pub file_path: PathBuf,
    /// Whether the checksum verification passed (no corrupt blocks).
    pub valid: bool,
    /// Total number of data blocks in the file.
    pub total_blocks: usize,
    /// Number of blocks with valid CRC32.
    pub verified_blocks: usize,
    /// Number of blocks with CRC32 mismatch.
    pub corrupt_blocks: usize,
    /// Total bytes of data verified.
    pub total_bytes: u64,
    /// Total bytes in corrupt blocks.
    pub corrupt_bytes: u64,
    /// Error messages (empty when valid).
    pub errors: Vec<String>,
}

/// Background data scrubber that verifies SSTable checksums.
pub struct DataScrubber {
    /// Directory containing SSTable files to scrub.
    sst_dir: String,
    /// Results of the most recent scrub cycle.
    results: Arc<Mutex<Vec<ScrubResult>>>,
    /// Flag to stop the background thread.
    stopped: Arc<AtomicBool>,
    /// Handle to the background thread.
    handle: Option<thread::JoinHandle<()>>,
}

impl DataScrubber {
    /// Create a new data scrubber targeting the given SSTable directory.
    pub fn new(sst_dir: impl Into<String>) -> Self {
        Self {
            sst_dir: sst_dir.into(),
            results: Arc::new(Mutex::new(Vec::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    /// Start the background scrubbing thread.
    ///
    /// The thread runs a scrub cycle every `interval`, then sleeps.
    /// Each cycle reads every `*.sst` file in the directory and verifies its
    /// CRC32 checksums.
    pub fn start_scrubbing(&mut self, interval: Duration) {
        let sst_dir = self.sst_dir.clone();
        let results = self.results.clone();
        let stopped = self.stopped.clone();

        self.handle = Some(thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                // Run one scrub cycle
                let cycle_results = scrub_sst_directory(&sst_dir);
                if let Ok(scrub_results) = cycle_results {
                    let mut res = results.lock().unwrap();
                    *res = scrub_results;
                }

                // Sleep, checking periodically for stop signal.
                for _ in 0..10 {
                    if stopped.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(interval / 10);
                }
            }
        }));
    }

    /// Stop the background scrubbing thread.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Returns the results of the most recent scrub cycle.
    pub fn results(&self) -> Vec<ScrubResult> {
        let res = self.results.lock().unwrap();
        res.clone()
    }
}

/// Scrub all `*.sst` files in the given directory by reading them and checking
/// CRC32 checksums.
fn scrub_sst_directory(dir: &str) -> std::result::Result<Vec<ScrubResult>, String> {
    let path = Path::new(dir);
    let mut results = Vec::new();

    let entries =
        std::fs::read_dir(path).map_err(|e| format!("cannot read directory '{}': {}", dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("readdir error: {}", e))?;
        let file_path = entry.path();

        if file_path.extension().and_then(|s| s.to_str()) != Some("sst") {
            continue;
        }

        let result = scrub_file(&file_path);
        results.push(result);
    }

    Ok(results)
}

/// Scrub a single SSTable file, validating the magic number and verifying CRC32
/// of all data blocks.
///
/// ## Format
///
/// Reads the SSTable V2 format:
/// - Validates the 8-byte magic number (`LSMSST03` for unencrypted)
/// - Reads the 8-byte footer at the end of the file to locate the meta block
/// - Deserializes the meta block to obtain per-block metadata (offset, size)
/// - For each data block, reads the compressed data + its 4-byte CRC32 trailer
///   and verifies the CRC32 matches the stored value
///
/// Encrypted SSTables (`LSMSST04`) are reported as invalid because the scrubber
/// does not have access to the encryption key to read the meta block.
pub fn scrub_file<P: AsRef<Path>>(path: P) -> ScrubResult {
    use std::io::{Read, Seek, SeekFrom};

    let path = path.as_ref();
    let file_path = path.to_path_buf();

    // Helper to build an error result early
    let error_result = |msg: String| -> ScrubResult {
        ScrubResult {
            file_path: file_path.clone(),
            valid: false,
            total_blocks: 0,
            verified_blocks: 0,
            corrupt_blocks: 0,
            total_bytes: 0,
            corrupt_bytes: 0,
            errors: vec![msg],
        }
    };

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return error_result(format!("Failed to open file: {}", e)),
    };

    let file_len = match file.metadata() {
        Ok(m) => m.len(),
        Err(e) => return error_result(format!("Failed to get file metadata: {}", e)),
    };

    // Minimum size: magic (8) + at least one data block (1) + footer (8)
    if file_len < 17 {
        return error_result("File too small to contain valid SSTable".to_string());
    }

    // Read and validate magic number
    let mut magic = [0u8; 8];
    if file.read_exact(&mut magic).is_err() {
        return error_result("Failed to read magic number".to_string());
    }

    if &magic != b"LSMSST03" && &magic != b"LSMSST04" {
        return error_result(format!(
            "Invalid magic number: expected LSMSST03 or LSMSST04, got {:?}",
            magic
        ));
    }

    // Encrypted SSTables require the encryption key to read the meta block
    if &magic == b"LSMSST04" {
        return error_result(
            "Cannot verify CRC32 of encrypted SSTable without encryption key".to_string(),
        );
    }

    // Read footer (last 8 bytes) to get meta block offset
    if file.seek(SeekFrom::End(-8)).is_err() {
        return error_result("Failed to seek to footer".to_string());
    }

    let mut footer_bytes = [0u8; 8];
    if file.read_exact(&mut footer_bytes).is_err() {
        return error_result("Failed to read footer".to_string());
    }

    let meta_offset = u64::from_le_bytes(footer_bytes);

    // Validate meta offset: must be within bounds and leave room for footer
    if meta_offset >= file_len - 8 {
        return error_result(format!(
            "Invalid meta block offset: {} (file length: {})",
            meta_offset, file_len
        ));
    }

    // Read compressed meta block
    let meta_size = (file_len - meta_offset - 8) as usize;
    if file.seek(SeekFrom::Start(meta_offset)).is_err() {
        return error_result("Failed to seek to meta block".to_string());
    }

    let mut meta_compressed = vec![0u8; meta_size];
    if file.read_exact(&mut meta_compressed).is_err() {
        return error_result("Failed to read meta block data".to_string());
    }

    // Decompress meta block
    let meta_decompressed = match lz4_flex::decompress_size_prepended(&meta_compressed) {
        Ok(d) => d,
        Err(e) => {
            return error_result(format!("Meta block decompression failed: {}", e));
        }
    };

    // Deserialize meta block (postcard format)
    let meta_block: MetaBlock = match crate::infra::codec::decode(&meta_decompressed) {
        Ok(m) => m,
        Err(e) => {
            return error_result(format!("Meta block deserialization failed: {}", e));
        }
    };

    let total_blocks = meta_block.blocks.len();
    let mut corrupt_blocks = 0usize;
    let mut total_bytes = 0u64;
    let mut corrupt_bytes = 0u64;
    let mut errors = Vec::new();

    for block in &meta_block.blocks {
        // block.size includes the 4-byte CRC32 trailer
        let data_size = (block.size as usize).saturating_sub(4);
        total_bytes += data_size as u64;

        if file.seek(SeekFrom::Start(block.offset)).is_err() {
            corrupt_blocks += 1;
            corrupt_bytes += data_size as u64;
            errors.push(format!(
                "Failed to seek to block at offset {}",
                block.offset
            ));
            continue;
        }

        let mut data = vec![0u8; data_size];
        if file.read_exact(&mut data).is_err() {
            corrupt_blocks += 1;
            corrupt_bytes += data_size as u64;
            errors.push(format!(
                "Failed to read block data at offset {} (size {})",
                block.offset, data_size
            ));
            continue;
        }

        let mut crc32_bytes = [0u8; 4];
        if file.read_exact(&mut crc32_bytes).is_err() {
            corrupt_blocks += 1;
            corrupt_bytes += data_size as u64;
            errors.push(format!(
                "Failed to read CRC32 trailer at offset {}",
                block.offset + data_size as u64
            ));
            continue;
        }

        let stored_crc32 = u32::from_le_bytes(crc32_bytes);
        let computed_crc32 = crc32fast::hash(&data);

        if stored_crc32 != computed_crc32 {
            corrupt_blocks += 1;
            corrupt_bytes += data_size as u64;
            errors.push(format!(
                "CRC32 mismatch at block offset {}: stored {:08x}, computed {:08x}",
                block.offset, stored_crc32, computed_crc32
            ));
        }
    }

    let verified_blocks = total_blocks - corrupt_blocks;

    ScrubResult {
        file_path,
        valid: corrupt_blocks == 0,
        total_blocks,
        verified_blocks,
        corrupt_blocks,
        total_bytes,
        corrupt_bytes,
        errors,
    }
}

/// Compare SSTable files on disk with tables tracked by the engine's VersionSet.
///
/// Returns scrub results for:
/// - **Orphan files**: `.sst` files on disk that have no corresponding Table in
///   VersionSet
/// - **Orphan tables**: Tables tracked by VersionSet whose `.sst` file is
///   missing from disk
pub fn scrub_with_version_set<C: Cache>(engine: &Engine<C>) -> Vec<ScrubResult> {
    let sst_dir = engine.sst_dir().clone();
    let core = engine.lock_core();
    let mut results = Vec::new();

    // Collect all file paths tracked by the VersionSet across all column families
    let mut tracked_paths: Vec<PathBuf> = Vec::new();
    for cf in core.version_set().column_families() {
        for table in core.version_set().get_tables(&cf) {
            if let Some(ref path) = table.path {
                tracked_paths.push(path.clone());
            }
        }
    }
    drop(core);

    // Scan disk for .sst files
    let disk_entries = match std::fs::read_dir(&sst_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return vec![ScrubResult {
                file_path: sst_dir,
                valid: false,
                total_blocks: 0,
                verified_blocks: 0,
                corrupt_blocks: 0,
                total_bytes: 0,
                corrupt_bytes: 0,
                errors: vec![format!("Failed to read SSTable directory: {}", e)],
            }];
        }
    };

    let disk_paths: Vec<PathBuf> = disk_entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sst"))
        .collect();

    // Detect orphan files: on disk but not tracked by VersionSet
    for disk_path in &disk_paths {
        if !tracked_paths.contains(disk_path) {
            let file_size = std::fs::metadata(disk_path).map(|m| m.len()).unwrap_or(0);
            results.push(ScrubResult {
                file_path: disk_path.clone(),
                valid: false,
                total_blocks: 0,
                verified_blocks: 0,
                corrupt_blocks: 0,
                total_bytes: file_size,
                corrupt_bytes: 0,
                errors: vec!["Orphan SSTable file: not tracked by VersionSet".to_string()],
            });
        }
    }

    // Detect orphan tables: tracked by VersionSet but file missing from disk
    for tracked_path in &tracked_paths {
        if !tracked_path.exists() {
            results.push(ScrubResult {
                file_path: tracked_path.clone(),
                valid: false,
                total_blocks: 0,
                verified_blocks: 0,
                corrupt_blocks: 0,
                total_bytes: 0,
                corrupt_bytes: 0,
                errors: vec!["Orphan table: SSTable file not found on disk".to_string()],
            });
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::log_record::LogRecord;
    use crate::infra::config::LsmConfig;
    use crate::storage::builder::SstableBuilder;
    use crate::storage::cache::NoopCache;
    use std::io::{Seek, Write};
    use std::time::Duration;

    // ── DataScrubber tests ─────────────────────────────────────────────────

    #[test]
    fn test_scrub_empty_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut scrubber = DataScrubber::new(dir.path().to_str().unwrap());
        scrubber.start_scrubbing(Duration::from_millis(50));
        std::thread::sleep(Duration::from_millis(150));
        scrubber.stop();

        let results = scrubber.results();
        assert!(results.is_empty(), "no .sst files → empty results");
    }

    #[test]
    fn test_scrub_bad_magic_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let sst_path = dir.path().join("test.sst");

        // Write a file with invalid magic.
        let mut f = std::fs::File::create(&sst_path).unwrap();
        f.write_all(b"APXSTORE").unwrap(); // not LSMSST03
        f.write_all(&[2u8]).unwrap();
        f.write_all(b"some payload data here").unwrap();
        f.flush().unwrap();

        let result = scrub_file(&sst_path);
        assert!(!result.valid, "file with invalid magic should fail scrub");
        assert!(result
            .errors
            .iter()
            .any(|e| e.contains("Invalid magic number")));
    }

    #[test]
    fn test_scrub_too_small_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let sst_path = dir.path().join("bad.sst");

        // Write a file that's too small (only 4 bytes).
        let mut f = std::fs::File::create(&sst_path).unwrap();
        f.write_all(b"BAD!").unwrap();
        f.flush().unwrap();

        let result = scrub_file(&sst_path);
        assert!(!result.valid, "corrupted .sst file should fail scrub");
        assert!(result.errors.iter().any(|e| e.contains("File too small")));
    }

    // ── CRC32 validity tests ───────────────────────────────────────────────

    #[test]
    fn test_scrub_crc32_valid() {
        let dir = tempfile::TempDir::new().unwrap();
        let sst_path = dir.path().join("valid.sst");

        // Build a proper SSTable with SstableBuilder.
        // Disable encryption explicitly because the scrubber doesn't
        // support encrypted SSTables (LSMSST04 magic).
        let config = crate::infra::config::StorageConfig::default();
        let enc_config = crate::storage::encryption::EncryptionConfig {
            enabled: false,
            key: [0u8; 32],
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut builder =
            SstableBuilder::new_with_encryption(sst_path.clone(), config, timestamp, &enc_config)
                .unwrap();

        builder
            .add(
                b"key1",
                &LogRecord::new(b"key1".to_vec(), b"value1".to_vec()),
            )
            .unwrap();
        builder
            .add(
                b"key2",
                &LogRecord::new(b"key2".to_vec(), b"value2".to_vec()),
            )
            .unwrap();
        let path = builder.finish().unwrap();

        let result = scrub_file(&path);
        assert!(result.valid, "valid SSTable should pass CRC32 check");
        assert!(result.errors.is_empty());
        assert!(
            result.total_blocks > 0,
            "should have at least one data block"
        );
        assert_eq!(result.corrupt_blocks, 0);
        assert_eq!(
            result.verified_blocks, result.total_blocks,
            "all blocks should be verified"
        );
        assert!(result.total_bytes > 0, "should have verified some bytes");
    }

    #[test]
    fn test_scrub_crc32_corrupt() {
        let dir = tempfile::TempDir::new().unwrap();
        let sst_path = dir.path().join("corrupt.sst");

        // Build a proper SSTable.
        // Disable encryption explicitly because the scrubber doesn't
        // support encrypted SSTables (LSMSST04 magic).
        let config = crate::infra::config::StorageConfig::default();
        let enc_config = crate::storage::encryption::EncryptionConfig {
            enabled: false,
            key: [0u8; 32],
        };
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut builder =
            SstableBuilder::new_with_encryption(sst_path.clone(), config, timestamp, &enc_config)
                .unwrap();

        builder
            .add(
                b"key1",
                &LogRecord::new(b"key1".to_vec(), b"value1".to_vec()),
            )
            .unwrap();
        builder
            .add(
                b"key2",
                &LogRecord::new(b"key2".to_vec(), b"value2".to_vec()),
            )
            .unwrap();
        let path = builder.finish().unwrap();

        // Corrupt the first data block by writing garbage after the magic
        {
            use std::io::SeekFrom;
            let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            // Overwrite bytes starting at offset 8 (right after magic) with garbage
            let garbage = vec![0xFF; 30];
            file.seek(SeekFrom::Start(8)).unwrap();
            file.write_all(&garbage).unwrap();
        }

        let result = scrub_file(&path);
        assert!(!result.valid, "corrupted SSTable should fail CRC32 check");
        assert!(
            result.corrupt_blocks > 0,
            "should have detected corrupt blocks"
        );
        assert!(result.corrupt_bytes > 0, "corrupt_bytes should be > 0");
        assert!(!result.errors.is_empty());
        assert!(
            result.errors.iter().any(|e| e.contains("CRC32 mismatch")),
            "error message should mention CRC32 mismatch"
        );
    }

    // ── Orphan detection tests (require engine integration) ────────────────

    #[test]
    fn test_scrub_orphan_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let sst_dir = dir.path().join("sstables");
        std::fs::create_dir_all(&sst_dir).unwrap();

        // Create the engine FIRST so discover_sstables_from_disk won't pick up
        // the file we're about to create
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Engine::new_from_config(&config, NoopCache).unwrap();

        // Now create an orphan .sst file AFTER engine init
        let orphan_path = sst_dir.join("orphan.sst");
        {
            let mut f = std::fs::File::create(&orphan_path).unwrap();
            f.write_all(b"LSMSST03").unwrap();
            f.write_all(&[0u8; 20]).unwrap();
        }

        let results = scrub_with_version_set(&engine);

        // Should detect the orphan file
        let orphan_results: Vec<&ScrubResult> = results
            .iter()
            .filter(|r| r.file_path == orphan_path)
            .collect();
        assert_eq!(orphan_results.len(), 1, "should find orphan .sst file");
        assert!(!orphan_results[0].valid);
        assert!(
            orphan_results[0]
                .errors
                .iter()
                .any(|e| e.contains("Orphan SSTable")),
            "orphan file error should mention 'Orphan SSTable'"
        );
    }

    #[test]
    fn test_scrub_orphan_table() {
        use crate::core::table::Table;
        use std::collections::BTreeMap;

        let dir = tempfile::TempDir::new().unwrap();
        let sst_dir = dir.path().join("sstables");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let engine = Engine::new_from_config(&config, NoopCache).unwrap();

        // Manually add a table with a path that doesn't exist
        let fake_path = sst_dir.join("nonexistent.sst");
        let orphan_table = Table {
            data: BTreeMap::new(),
            level: 0,
            path: Some(fake_path.clone()),
            min_key: b"a".to_vec(),
            max_key: b"z".to_vec(),
            bloom_filter: None,
        };

        {
            let mut core = engine.lock_core_mut();
            core.version_set_mut().add_table("default", orphan_table);
        }

        let results = scrub_with_version_set(&engine);

        // Should detect the orphan table
        let table_results: Vec<&ScrubResult> = results
            .iter()
            .filter(|r| r.file_path == fake_path)
            .collect();
        assert_eq!(
            table_results.len(),
            1,
            "should detect orphan table with missing file"
        );
        assert!(!table_results[0].valid);
        assert!(
            table_results[0]
                .errors
                .iter()
                .any(|e| e.contains("Orphan table")),
            "orphan table error should mention 'Orphan table'"
        );
    }
}
