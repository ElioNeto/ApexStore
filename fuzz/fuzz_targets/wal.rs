#![no_main]

use libfuzzer_sys::fuzz_target;
use apexstore::core::log_record::LogRecord;
use apexstore::storage::wal::WriteAheadLog;
use apexstore::storage::encryption::EncryptionConfig;
use tempfile::TempDir;

/// Disable encryption so the fuzzer focuses on WAL frame logic
/// rather than AES-GCM encryption/decryption roundtrips.
fn no_encryption() -> EncryptionConfig {
    EncryptionConfig {
        enabled: false,
        key: [0u8; 32],
    }
}

fuzz_target!(|data: &[u8]| {
    // Guard against empty input — WAL operations need at least some data
    if data.is_empty() {
        return;
    }

    // Create a temp directory and WAL file
    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };

    // Create WAL with disabled encryption
    let wal = match WriteAheadLog::new_with_encryption(
        dir.path(),
        "default",
        &no_encryption(),
    ) {
        Ok(w) => w,
        Err(_) => return,
    };

    // ── Test 1: Write a single record with fuzzed key and value ──
    // Split the fuzzed data into key and value halves.
    let mid = data.len() / 2;
    let key = &data[..mid];
    let val = &data[mid..];

    let record = LogRecord::new(key.to_vec(), val.to_vec());
    let _ = wal.write_record(&record);

    // ── Test 2: Write a batch with multiple records ──
    // Use the first half as key1, a fixed key for key2.
    let records = vec![
        LogRecord::new(key.to_vec(), val.to_vec()),
        LogRecord::new(
            b"batch_key".to_vec(),
            val.to_vec(),
        ),
    ];
    let _ = wal.write_batch(&records);

    // ── Test 3: Write a tombstone with fuzzed key ──
    let tombstone = LogRecord::tombstone(key.to_vec());
    let _ = wal.write_record(&tombstone);

    // ── Test 4: Try recovery ──
    // Recovery should never panic, even with corrupted data.
    let _recovered = wal.recover();

    // ── Test 5: Write a record with TTL ──
    let ttl_record = LogRecord::new_with_ttl(
        key.to_vec(),
        val.to_vec(),
        std::time::Duration::from_secs(3600),
    );
    let _ = wal.write_record(&ttl_record);

    // ── Test 6: Sync and check size ──
    let _ = wal.sync();
    let _ = wal.size();
});
