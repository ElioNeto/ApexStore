#![no_main]

use libfuzzer_sys::fuzz_target;
use apexstore::core::log_record::LogRecord;
use apexstore::infra::config::StorageConfig;
use apexstore::storage::builder::SstableBuilder;
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::storage::encryption::EncryptionConfig;
use apexstore::storage::reader::SstableReader;
use tempfile::TempDir;

/// Disable encryption so the fuzzer focuses on SSTable block format,
/// decompression, and indexing logic.
fn no_encryption() -> EncryptionConfig {
    EncryptionConfig {
        enabled: false,
        key: [0u8; 32],
    }
}

/// Build a small SSTable with one record and try various read operations
/// using the fuzzed data.
fn fuzz_sstable_roundtrip(data: &[u8]) {
    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(_) => return,
    };
    let sst_path = dir.path().join("fuzz.sst");

    let config = StorageConfig::default();
    let enc_cfg = no_encryption();
    let timestamp = match std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
    {
        Ok(d) => d.as_nanos(),
        Err(_) => return,
    };

    // ── Build an SSTable with a single record ──
    // We use a fixed key to guarantee strictly-increasing order.
    let mut builder = match SstableBuilder::new_with_encryption(
        sst_path.clone(),
        config.clone(),
        timestamp,
        &enc_cfg,
    ) {
        Ok(b) => b,
        Err(_) => return,
    };

    let record = LogRecord::new(b"fuzz_key".to_vec(), data.to_vec());
    if builder.add(b"fuzz_key", &record).is_err() {
        return;
    }

    // Try adding a second record with a larger key so ordering is maintained.
    let record2 = LogRecord::new(b"fuzz_key2".to_vec(), data.to_vec());
    let _ = builder.add(b"fuzz_key2", &record2);

    let path = match builder.finish() {
        Ok(p) => p,
        Err(_) => return,
    };

    // ── Open and read back ──
    let cache = GlobalBlockCache::new(100, 4096);
    let reader = match SstableReader::open(path, config, cache) {
        Ok(r) => r,
        Err(_) => return,
    };

    // Test 1: Read the known key
    let _ = reader.get(b"fuzz_key");

    // Test 2: Read using fuzzed data as lookup key
    // This exercises Bloom filter + block lookup with arbitrary keys.
    let _ = reader.get(data);

    // Test 3: Read another known key
    let _ = reader.get(b"fuzz_key2");

    // Test 4: Scan all records
    let _ = reader.scan();

    // Test 5: Check Bloom filter for known key and fuzzed key
    let _ = reader.might_contain(b"fuzz_key");
    let _ = reader.might_contain(data);
}

fuzz_target!(|data: &[u8]| {
    // Guard against empty input — need at least 1 byte for a value.
    if data.is_empty() {
        return;
    }

    fuzz_sstable_roundtrip(data);
});
