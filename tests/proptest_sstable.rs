use apexstore::core::log_record::LogRecord;
use apexstore::infra::config::StorageConfig;
use apexstore::storage::builder::SstableBuilder;
use apexstore::storage::cache::GlobalBlockCache;
use apexstore::storage::encryption::EncryptionConfig;
use apexstore::storage::reader::SstableReader;
use proptest::prelude::*;
use tempfile::TempDir;

/// Disable encryption explicitly so the test is not affected by encryption
/// roundtrip bugs.  Encryption roundtrip should be tested separately.
fn no_encryption() -> EncryptionConfig {
    EncryptionConfig {
        enabled: false,
        key: [0u8; 32],
    }
}

/// Generate key-value pairs where keys are already sorted.  The
/// SstableBuilder requires strictly increasing key order (it validates
/// this internally).  This strategy generates pairs with keys that
/// are already in the right order.
fn sorted_records() -> impl Strategy<Value = Vec<(Vec<u8>, Vec<u8>)>> {
    proptest::collection::vec(
        (
            proptest::collection::vec(proptest::arbitrary::any::<u8>(), 1..=16),
            proptest::collection::vec(proptest::arbitrary::any::<u8>(), 1..=64),
        ),
        1..=5,
    )
    .prop_map(|mut pairs| {
        // Sort by key to satisfy SstableBuilder's sorted-key invariant.
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    })
}

proptest! {
    #[test]
    fn test_sstable_write_read_roundtrip(records in sorted_records()) {
        let dir = TempDir::new().unwrap();
        let sst_path = dir.path().join("test.sst");

        let config = StorageConfig::default();
        let enc_cfg = no_encryption();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut builder = SstableBuilder::new_with_encryption(
            sst_path.clone(), config.clone(), timestamp, &enc_cfg,
        ).unwrap();

        for (k, v) in &records {
            let record = LogRecord::new(k.clone(), v.clone());
            builder.add(k, &record).unwrap();
        }
        builder.finish().unwrap();

        // Read back and verify
        let cache = GlobalBlockCache::new(100, 4096);
        let reader = SstableReader::open_with_encryption(
            sst_path, config, cache, &enc_cfg,
        ).unwrap();
        for (k, v) in &records {
            let result = reader.get(k).unwrap();
            prop_assert_eq!(
                result.as_ref().map(|lr| &lr.value),
                Some(v),
                "key {:?} should have value {:?} but got {:?}",
                k, v, result.as_ref().map(|lr| &lr.value)
            );
        }

        // Verify total count
        let all = reader.scan().unwrap();
        prop_assert_eq!(all.len(), records.len(), "should have {} entries but got {}", records.len(), all.len());
    }
}
