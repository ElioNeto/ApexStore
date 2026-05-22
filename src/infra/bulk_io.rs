//! Bulk import/export for ApexStore — high-throughput data migration.
//!
//! Supports JSON (streaming via serde) and CSV (streaming via csv crate).
//!
//! # Streaming
//!
//! All functions stream data through paginated engine scans (export) or
//! batched writes (import) so that arbitrarily large datasets can be
//! processed without loading everything into memory.
//!
//! ## JSON format (export)
//!
//! ```json
//! [{"key":"k1","value":"v1"},{"key":"k2","value":"v2"}]
//! ```
//!
//! ## JSON format (import)
//!
//! Array of objects with `key` and `value` fields:
//! ```json
//! [{"key":"k1","value":"v1"},{"key":"k2","value":"v2"}]
//! ```
//!
//! ## CSV format
//!
//! ```csv
//! key,value
//! k1,v1
//! k2,v2
//! ```

use crate::core::engine::Engine;
use crate::infra::error::{LsmError, Result};
use crate::storage::cache::Cache;
use serde::de::{self, SeqAccess, Visitor};
use serde::Deserializer;
use serde::Deserialize;
use serde_json::Value;
use std::io::{Read, Write};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of records per scan page when exporting.
const EXPORT_PAGE_SIZE: usize = 2000;

/// Number of records per `set_batch_cf` call when importing.
const IMPORT_BATCH_SIZE: usize = 500;

// ---------------------------------------------------------------------------
// Progress callback
// ---------------------------------------------------------------------------

/// Progress callback: receives `(items_processed, total_items)`.
///
/// `total_items` may be `0` when the total is unknown (e.g. during streaming
/// import where the total record count isn't known upfront).
pub type ProgressFn = Box<dyn Fn(u64, u64) + Send + Sync>;

// ---------------------------------------------------------------------------
// Helper: paginated scan with exclusive lower bound
// ---------------------------------------------------------------------------

/// Compute the byte sequence immediately after `key` so it can be used as an
/// exclusive lower bound for pagination.
///
/// Returns `None` when `key` consists entirely of `0xFF` bytes — in that case
/// there is no representable key "after" it.
fn key_after(key: &[u8]) -> Option<Vec<u8>> {
    let mut result = key.to_vec();
    for i in (0..result.len()).rev() {
        if result[i] < 0xFF {
            result[i] += 1;
            return Some(result);
        }
        result[i] = 0;
    }
    // Every byte was 0xFF — extend with a 0 byte to create a valid successor.
    result.push(0);
    Some(result)
}

/// Iterate over all key-value pairs in a column family using paginated scans.
///
/// The closure receives `(key, value)` and returns `Ok(true)` to continue or
/// `Ok(false)` to stop early.
fn for_each_kv<C: Cache>(
    engine: &Engine<C>,
    cf: &str,
    mut f: impl FnMut(&[u8], &[u8]) -> Result<bool>,
) -> Result<()> {
    let mut lower: Option<Vec<u8>> = None;

    loop {
        let results = engine.scan_cf(cf, lower.as_deref(), None, Some(EXPORT_PAGE_SIZE))?;
        if results.is_empty() {
            break;
        }

        for (key, value) in &results {
            if !f(key, value)? {
                return Ok(());
            }
        }

        // Determine if there are more pages.
        if results.len() < EXPORT_PAGE_SIZE {
            break;
        }
        match results.last() {
            Some((last_key, _)) => match key_after(last_key) {
                Some(next) => lower = Some(next),
                None => break,
            },
            None => break,
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JsonKvPair {
    key: String,
    value: String,
}

/// Stream-parse a JSON array of `{"key": ..., "value": ...}` objects.
///
/// Uses serde's `SeqAccess` visitor so that elements are yielded one at a time
/// without loading the entire file into memory.
fn stream_json_array<R: Read, F: FnMut(Value) -> Result<bool>>(
    reader: R,
    f: F,
) -> Result<()> {
    struct CallbackVisitor<F>(F);

    impl<'de, F: FnMut(Value) -> Result<bool>> Visitor<'de> for CallbackVisitor<F> {
        type Value = ();

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a JSON array")
        }

        fn visit_seq<A>(mut self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            loop {
                match seq.next_element::<Value>() {
                    Ok(Some(item)) => {
                        // Use `&mut self.0` to call FnMut without consuming it
                        let cont = (self.0)(item).map_err(de::Error::custom)?;
                        if !cont {
                            return Ok(());
                        }
                    }
                    Ok(None) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    let mut de = serde_json::Deserializer::from_reader(reader);
    de.deserialize_any(CallbackVisitor(f))
        .map_err(LsmError::JsonError)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API — export
// ---------------------------------------------------------------------------

/// Export all key-value pairs from a column family as a JSON array.
///
/// The output is a streaming JSON array written to `writer`.  The array is
/// written element-by-element so memory usage stays constant regardless of
/// dataset size.
pub fn export_json<C: Cache, W: Write>(
    engine: &Engine<C>,
    writer: &mut W,
    cf: Option<&str>,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let cf = cf.unwrap_or("default");
    let mut first = true;
    let mut count = 0u64;

    writer.write_all(b"[")?;

    for_each_kv(engine, cf, |key, value| {
        if !first {
            writer.write_all(b",")?;
        }
        first = false;

        let key_str = String::from_utf8_lossy(key);
        let val_str = String::from_utf8_lossy(value);

        write!(
            writer,
            "{{\"key\":{},\"value\":{}}}",
            serde_json::to_string(&key_str).map_err(LsmError::JsonError)?,
            serde_json::to_string(&val_str).map_err(LsmError::JsonError)?,
        )?;

        count += 1;
        if count.is_multiple_of(EXPORT_PAGE_SIZE as u64) {
            if let Some(ref cb) = progress {
                cb(count, 0);
            }
        }

        Ok(true)
    })?;

    writer.write_all(b"]")?;

    if let Some(ref cb) = progress {
        cb(count, count);
    }

    Ok(())
}

/// Export all key-value pairs from a column family as CSV.
///
/// Writes a header row `key,value` followed by data rows.  Streams data using
/// paginated engine scans.
pub fn export_csv<C: Cache, W: Write>(
    engine: &Engine<C>,
    writer: &mut W,
    cf: Option<&str>,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let cf = cf.unwrap_or("default");
    let mut wtr = csv::Writer::from_writer(writer);
    let mut count = 0u64;

    // Write header
    wtr.write_record(["key", "value"])
        .map_err(|e| LsmError::InvalidArgument(format!("CSV write error: {}", e)))?;

    for_each_kv(engine, cf, |key, value| {
        let key_str = String::from_utf8_lossy(key);
        let val_str = String::from_utf8_lossy(value);

        wtr.write_record([key_str.as_ref(), val_str.as_ref()])
            .map_err(|e| LsmError::InvalidArgument(format!("CSV write error: {}", e)))?;

        count += 1;
        if count.is_multiple_of(EXPORT_PAGE_SIZE as u64) {
            if let Some(ref cb) = progress {
                cb(count, 0);
            }
        }

        Ok(true)
    })?;

    wtr.flush().map_err(|e| LsmError::InvalidArgument(format!("CSV flush error: {}", e)))?;

    if let Some(ref cb) = progress {
        cb(count, count);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API — import
// ---------------------------------------------------------------------------

/// Import key-value pairs from a JSON array.
///
/// Expects the input to be a JSON array of objects with `key` and `value`
/// string fields:
///
/// ```json
/// [{"key":"k1","value":"v1"}, {"key":"k2","value":"v2"}]
/// ```
///
/// Records are inserted in batches via `set_batch_cf` for atomicity and
/// performance.
pub fn import_json<C: Cache, R: Read>(
    engine: &Engine<C>,
    reader: R,
    cf: Option<&str>,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let cf = cf.unwrap_or("default");
    let mut count = 0u64;
    let mut batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(IMPORT_BATCH_SIZE);

    stream_json_array(reader, |item| {
        let pair = serde_json::from_value::<JsonKvPair>(item)
            .map_err(|e| LsmError::InvalidArgument(format!("Invalid JSON entry: {}", e)))?;

        batch.push((pair.key.into_bytes(), pair.value.into_bytes()));

        if batch.len() >= IMPORT_BATCH_SIZE {
            engine.set_batch_cf(cf, &batch)?;
            count += batch.len() as u64;
            batch.clear();
            if let Some(ref cb) = progress {
                cb(count, 0);
            }
        }

        Ok(true)
    })?;

    // Flush remaining batch
    if !batch.is_empty() {
        engine.set_batch_cf(cf, &batch)?;
        count += batch.len() as u64;
    }

    if let Some(ref cb) = progress {
        cb(count, count);
    }

    Ok(())
}

/// Import key-value pairs from a CSV file.
///
/// Expects a header row with at least `key` and `value` columns.
/// Additional columns are ignored.
///
/// Records are inserted in batches via `set_batch_cf` for atomicity and
/// performance.  The CSV reader streams records one at a time.
pub fn import_csv<C: Cache, R: Read>(
    engine: &Engine<C>,
    reader: R,
    cf: Option<&str>,
    progress: Option<ProgressFn>,
) -> Result<()> {
    let cf = cf.unwrap_or("default");
    let mut rdr = csv::Reader::from_reader(reader);
    let mut count = 0u64;
    let mut batch: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(IMPORT_BATCH_SIZE);

    // Determine column indices for "key" and "value".
    let headers = rdr
        .headers()
        .map_err(|e| LsmError::InvalidArgument(format!("CSV header error: {}", e)))?
        .clone();

    let key_idx = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("key"))
        .ok_or_else(|| {
            LsmError::InvalidArgument(
                "CSV must have a 'key' column".to_string(),
            )
        })?;

    let val_idx = headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case("value"))
        .ok_or_else(|| {
            LsmError::InvalidArgument(
                "CSV must have a 'value' column".to_string(),
            )
        })?;

    for result in rdr.records() {
        let record = result
            .map_err(|e| LsmError::InvalidArgument(format!("CSV read error: {}", e)))?;

        let key = record
            .get(key_idx)
            .ok_or_else(|| {
                LsmError::InvalidArgument("Missing key field in CSV row".to_string())
            })?
            .as_bytes()
            .to_vec();

        let value = record
            .get(val_idx)
            .ok_or_else(|| {
                LsmError::InvalidArgument("Missing value field in CSV row".to_string())
            })?
            .as_bytes()
            .to_vec();

        batch.push((key, value));

        if batch.len() >= IMPORT_BATCH_SIZE {
            engine.set_batch_cf(cf, &batch)?;
            count += batch.len() as u64;
            batch.clear();
            if let Some(ref cb) = progress {
                cb(count, 0);
            }
        }
    }

    // Flush remaining batch
    if !batch.is_empty() {
        engine.set_batch_cf(cf, &batch)?;
        count += batch.len() as u64;
    }

    if let Some(ref cb) = progress {
        cb(count, count);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::LsmConfig;
    use crate::storage::cache::GlobalBlockCache;
    use std::sync::Arc;
    use tempfile::tempdir;

    type TestEngine = Engine<Arc<GlobalBlockCache>>;

    /// Helper: create engine + temp dir. Keep both alive for the test scope.
    struct TestContext {
        engine: TestEngine,
        _dir: tempfile::TempDir,
    }

    fn setup_engine() -> TestContext {
        let dir = tempdir().unwrap();
        let mut config = LsmConfig::default();
        config.core.dir_path = dir.path().to_path_buf();
        let cache = GlobalBlockCache::new(100, 4096);
        let engine = Engine::new_from_config(&config, cache).unwrap();
        TestContext {
            engine,
            _dir: dir,
        }
    }

    fn put(engine: &TestEngine, cf: &str, k: &str, v: &str) {
        engine
            .put_cf(cf, k.as_bytes().to_vec(), v.as_bytes().to_vec())
            .unwrap();
    }

    #[test]
    fn test_export_json_basic() {
        let ctx = setup_engine();
        put(&ctx.engine, "default", "a", "1");
        put(&ctx.engine, "default", "b", "2");

        let mut buf = Vec::new();
        export_json(&ctx.engine, &mut buf, None, None).unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert!(output.starts_with('['));
        assert!(output.ends_with(']'));
        assert!(output.contains("\"key\":\"a\""));
        assert!(output.contains("\"value\":\"1\""));
        assert!(output.contains("\"key\":\"b\""));
        assert!(output.contains("\"value\":\"2\""));
    }

    #[test]
    fn test_export_json_empty() {
        let ctx = setup_engine();
        let mut buf = Vec::new();
        export_json(&ctx.engine, &mut buf, None, None).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "[]");
    }

    #[test]
    fn test_export_csv_basic() {
        let ctx = setup_engine();
        put(&ctx.engine, "default", "x", "10");
        put(&ctx.engine, "default", "y", "20");

        let mut buf = Vec::new();
        export_csv(&ctx.engine, &mut buf, None, None).unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("key,value"));
        assert!(output.contains("x,10"));
        assert!(output.contains("y,20"));
    }

    #[test]
    fn test_export_csv_empty() {
        let ctx = setup_engine();
        let mut buf = Vec::new();
        export_csv(&ctx.engine, &mut buf, None, None).unwrap();
        // Should have just the header when empty
        let header = String::from_utf8(buf).unwrap();
        assert!(
            header == "key,value\n" || header == "key,value\r\n",
            "expected header line, got: {:?}",
            header
        );
    }

    #[test]
    fn test_import_json_basic() {
        let ctx = setup_engine();

        let json = r#"[{"key":"k1","value":"v1"},{"key":"k2","value":"v2"}]"#;
        import_json(&ctx.engine, json.as_bytes(), None, None).unwrap();

        assert_eq!(ctx.engine.get("k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(ctx.engine.get("k2").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_import_json_cf() {
        let ctx = setup_engine();

        let json = r#"[{"key":"k1","value":"v1"}]"#;
        import_json(&ctx.engine, json.as_bytes(), Some("mycf"), None).unwrap();

        assert_eq!(ctx.engine.get("k1").unwrap(), None);
        assert_eq!(
            ctx.engine.get_cf("mycf", "k1").unwrap(),
            Some(b"v1".to_vec())
        );
    }

    #[test]
    fn test_import_csv_basic() {
        let ctx = setup_engine();

        let csv_data = "key,value\nk1,v1\nk2,v2\n";
        import_csv(&ctx.engine, csv_data.as_bytes(), None, None).unwrap();

        assert_eq!(ctx.engine.get("k1").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(ctx.engine.get("k2").unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn test_import_csv_with_extra_columns() {
        let ctx = setup_engine();

        let csv_data = "key,value,ignored\nk1,v1,extra\nk2,v2,stuff\n";
        import_csv(&ctx.engine, csv_data.as_bytes(), None, None).unwrap();

        assert_eq!(ctx.engine.get("k1").unwrap(), Some(b"v1".to_vec()));
    }

    #[test]
    fn test_import_csv_missing_header() {
        let ctx = setup_engine();
        let csv_data = "k,v\nk1,v1\n";
        let result = import_csv(&ctx.engine, csv_data.as_bytes(), None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_export_import_roundtrip() {
        let ctx = setup_engine();

        // Insert data
        for i in 0..50 {
            let k = format!("key_{}", i);
            let v = format!("value_{}", i);
            put(&ctx.engine, "default", &k, &v);
        }

        // Export to JSON
        let mut json_buf = Vec::new();
        export_json(&ctx.engine, &mut json_buf, None, None).unwrap();

        // Import into a fresh CF
        import_json(&ctx.engine, json_buf.as_slice(), Some("restored"), None).unwrap();

        // Verify
        for i in 0..50 {
            let k = format!("key_{}", i);
            let v = format!("value_{}", i);
            assert_eq!(
                ctx.engine.get_cf("restored", k.as_bytes()).unwrap(),
                Some(v.into_bytes())
            );
        }
    }

    #[test]
    fn test_progress_callback() {
        let ctx = setup_engine();

        for i in 0..10 {
            let k = format!("key_{}", i);
            let v = format!("val_{}", i);
            put(&ctx.engine, "default", &k, &v);
        }

        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let calls_clone = calls.clone();
        let cb: ProgressFn = Box::new(move |current, total| {
            let mut c = calls_clone.lock().unwrap();
            c.push((current, total));
        });

        let mut buf = Vec::new();
        export_json(&ctx.engine, &mut buf, None, Some(cb)).unwrap();

        let c = calls.lock().unwrap();
        // Last call should have total == count
        assert!(!c.is_empty());
        let &(last_current, last_total) = c.last().unwrap();
        assert_eq!(last_current, 10);
        assert_eq!(last_total, 10);
    }

    #[test]
    fn test_key_after() {
        assert_eq!(key_after(b"abc"), Some(b"abd".to_vec()));
        assert_eq!(key_after(b"ab\xFF"), Some(b"ac\x00".to_vec()));
        // All-bytes-max: carry propagates through all bytes, then extends
        assert_eq!(key_after(b"\xFF\xFF"), Some(b"\x00\x00\x00".to_vec()));
    }

    #[test]
    fn test_import_json_large_batch() {
        let ctx = setup_engine();

        // Generate pairs that exceed IMPORT_BATCH_SIZE
        let mut pairs = Vec::new();
        for i in 0..IMPORT_BATCH_SIZE * 3 {
            pairs.push(format!(
                "{{\"key\":\"k{}\",\"value\":\"v{}\"}}",
                i, i
            ));
        }
        let json = format!("[{}]", pairs.join(","));

        import_json(&ctx.engine, json.as_bytes(), None, None).unwrap();

        for i in 0..IMPORT_BATCH_SIZE * 3 {
            let k = format!("k{}", i);
            let v = format!("v{}", i);
            assert_eq!(
                ctx.engine.get(k.as_bytes()).unwrap(),
                Some(v.into_bytes())
            );
        }
    }
}
