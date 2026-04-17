# Test Scenarios for Range Scan & Pagination

## API Tests

### GET /scan

#### Basic Range Scan
```bash
curl "http://localhost:8080/scan"
```
**Expected:** Returns all records (first 1000 by default) with `next_cursor: null`

#### Range with Start Key
```bash
curl "http://localhost:8080/scan?start_key=user:100"
```
**Expected:** Records starting from user:100 onwards

#### Range with End Key
```bash
curl "http://localhost:8080/scan?end_key=user:200"
```
**Expected:** Records before user:200 (exclusive)

#### Range with Both Bounds
```bash
curl "http://localhost:8080/scan?start_key=user:100&end_key=user:200"
```
**Expected:** Records in range [user:100, user:200)

#### Limited Results with Pagination
```bash
curl "http://localhost:8080/scan?limit=10"
# Get cursor from response
curl "http://localhost:8080/scan?start_key=<cursor>&limit=10"
```
**Expected:** First page returns 10 records with `next_cursor`, second page returns next 10 without overlap

#### Invalid Arguments
```bash
curl "http://localhost:8080/scan?limit=0"
curl "http://localhost:8080/scan?limit=20000"
curl "http://localhost:8080/scan?start_key=b&end_key=a"
```
**Expected:** 400 Bad Request responses with appropriate error messages

#### Exceeding Max Limit
```bash
curl "http://localhost:8080/scan?limit=10001"
```
**Expected:** 429 Too Many Requests

### GET /keys/search

#### Basic Prefix Search
```bash
curl "http://localhost:8080/keys/search?q=user:"
```
**Expected:** All keys matching "user:*" prefix (first 1000)

#### Limited Prefix Search with Pagination
```bash
curl "http://localhost:8080/keys/search?q=user:&limit=5"
# Get cursor and continue
curl "http://localhost:8080/keys/search?q=user:&limit=5&cursor=<cursor>"
```
**Expected:** Paginated results for prefix matches

#### Search Empty Prefix
```bash
curl "http://localhost:8080/keys/search?q=&limit=10"
```
**Expected:** First 10 keys in lexicographic order

## CLI Tests

### SCAN Command

```bash
# All records
$ SCAN

# Specific range
$ SCAN user:100 user:200

# Limited scan
$ SCAN user:100 user:200 50

# All with limit
$ SCAN "" "" 100
```

### KEYS Command

```bash
# List all keys
$ KEYS

# Keys with prefix
$ KEYS user:

# Keys with prefix and limit
$ KEYS product: 50
```

### PREFIX Command

```bash
# Equivalent to KEYS <prefix>
$ PREFIX user:

# With limit
$ PREFIX user: 100
```

## Engine Level Tests

### scan_range Tests

```rust
// Empty database
assert!(engine.scan_range(None, None, 100).unwrap().0.is_empty());

// Basic range
let (results, cursor) = engine.scan_range(None, None, 100)?;
assert_eq!(results.len(), expected_count);
assert!(cursor.is_none()); // No more results

// Range filter
let (results, _) = engine.scan_range(Some("user:50"), None, 100)?;
assert!(results.iter().all(|(k, _)| k >= "user:50"));

// End filter
let (results, _) = engine.scan_range(None, Some("user:50"), 100)?;
assert!(results.iter().all(|(k, _)| k < "user:50"));

// Limit enforcement
let (results, cursor) = engine.scan_range(None, None, 10)?;
assert_eq!(results.len(), 10);
assert!(cursor.is_some()); // There should be more

// Tombstone filtering
engine.set("user:001", b"value".to_vec())?;
engine.delete("user:001")?;
let (results, _) = engine.scan_range(None, None, 100)?;
assert!(!results.iter().any(|(k, _)| k == "user:001"));

// MemTable overrides SSTable
engine.set("user:001", b"v1".to_vec())?;
engine.flush()?;
engine.set("user:001", b"v2".to_vec())?;
let (results, _) = engine.scan_range(None, None, 100)?;
assert_eq!(results.iter().find(|(k, _)| k == "user:001").unwrap().1, b"v2");
```

### search_prefix Tests

```rust
// Empty results
let (results, _) = engine.search_prefix("nonexistent:", None, 100)?;
assert!(results.is_empty());

// Basic prefix match
let (results, _) = engine.search_prefix("user:", None, 100)?;
assert!(results.iter().all(|(k, _)| k.starts_with("user:")));

// Pagination
let (page1, cursor) = engine.search_prefix("user:", None, 5)?;
assert_eq!(page1.len(), 5);
let (page2, _) = engine.search_prefix("user:", cursor.as_deref(), 5)?;
assert_eq!(page2.len(), 5);
assert!(page2.iter().all(|(k, _)| k > cursor.as_ref().unwrap()));

// Tombstone filtering (same as scan_range)
// MemTable overrides SSTable (same as scan_range)
```

## Performance Tests

### Large Dataset Scan
```rust
#[test]
fn test_scan_100k_keys() {
    // Insert 100k keys
    for i in 0..100_000 {
        engine.set(format!("key:{}", i), vec![b'x'; 64]).unwrap();
    }
    engine.flush()?;

    // Range scan with limit should be fast
    let start = Instant::now();
    let (results, _) = engine.scan_range(None, None, 10)?;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 10);
    assert!(elapsed.as_millis() < 50); // Should return quickly
}

#[test]
fn test_prefix_scan_100k_keys() {
    // Insert keys with various prefixes
    for i in 0..50_000 {
        engine.set(format!("user:{}", i), vec![b'x'; 64]).unwrap();
    }
    for i in 0..50_000 {
        engine.set(format!("product:{}", i), vec![b'x'; 64]).unwrap();
    }
    engine.flush()?;

    // Prefix search with limit should be fast
    let start = Instant::now();
    let (results, _) = engine.search_prefix("user:", None, 10)?;
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 50000); // All user:* keys
    // With proper optimization, this should complete in reasonable time
    assert!(elapsed.as_millis() < 1000);
}
```

## Edge Cases

### Boundary Conditions
```rust
// Exact start boundary
let (results, _) = engine.scan_range(Some("user:000"), None, 100)?;
assert_eq!(results[0].0, "user:000"); // First key

// Exact end boundary (exclusive)
let (results, _) = engine.scan_range(None, Some("user:001"), 100)?;
assert!(!results.iter().any(|(k, _)| k == "user:001"));

// Single key result
let (results, _) = engine.scan_range(Some("user:000"), Some("user:002"), 100)?;
assert_eq!(results.len(), 2); // user:000, user:001
```

### Concurrent Writes
```rust
// During pagination, new keys may be inserted
// No strong guarantee - at-most-once semantics
// Implementation should handle this gracefully
```

### Invalid Cursors
```rust
// Cursor from other dataset may return empty
let fake_cursor = Some("zzzzzzz".to_string());
let (results, _) = engine.scan_range(fake_cursor.as_deref(), None, 100)?;
assert!(results.is_empty() || all keys > "zzzzzzz");
```

### Empty Results
```rust
// No results in range
let (results, _) = engine.scan_range(Some("zzz"), None, 100)?;
assert!(results.is_empty());
assert!(next_cursor.is_none()); // Should be null, not set to empty string

// Limit zero
assert!(engine.scan_range(None, None, 0).is_err());

// Start >= end
assert!(engine.scan_range(Some("z"), Some("a"), 100).is_err());
assert!(engine.scan_range(Some("a"), Some("a"), 100).is_err());
```

## Integration Flow

### API Client Workflow
```javascript
// Client code pattern
async function scanAll(params = {}) {
    let results = [];
    let startKey = params.startKey || null;
    let endKey = params.endKey || null;
    let limit = params.limit || 1000;

    do {
        const response = await api.scan({
            start_key: startKey,
            end_key: endKey,
            limit: limit
        });

        results.push(...response.data);

        if (response.next_cursor) {
            startKey = response.next_cursor;
        } else {
            break;
        }
    } while (true);

    return results;
}
```

### CLI Pagination Pattern
```bash
# User types SCAN user: 5
# CLI internally fetches:
#   Page 1: scan_range(Some("user:"), None, 5)
#   Page 2: scan_range(Some(last_key_from_page1), None, 5)
#   ... until no more results

# User types KEYS user: 50
# Similarly paginates automatically
```
