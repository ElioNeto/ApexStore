# Changelog v1.5 - CLI Command Parity

## Release Date
March 6, 2026

## Overview
This release equalizes the CLI command set with REST API endpoints, providing feature parity between both interfaces and enhancing the user experience.

## New Features

### 1. STATS ALL - Detailed Statistics
**Command:** `STATS ALL`

**Description:** Displays comprehensive statistics in JSON format including:
- MemTable records and size
- SSTable files, records, and disk usage
- WAL size
- Total record count
- Configuration parameters

**Example:**
```bash
lsm> STATS ALL
{
  "mem_records": 3,
  "mem_kb": 2,
  "sst_files": 2,
  "sst_records": 47,
  "sst_kb": 156,
  "wal_kb": 1,
  "total_records": 50,
  "memtable_max_size": 4
}
```

**REST API Equivalent:** `GET /stats/all`

### 2. SEARCH - Advanced Search with Prefix Support
**Command:** `SEARCH <query> [--prefix]`

**Description:** Searches for records matching a query with two modes:
- **Substring mode** (default): Finds all keys containing the query string
- **Prefix mode** (with `--prefix` flag): Finds all keys starting with the query string (optimized)

**Examples:**
```bash
# Substring search
lsm> SEARCH user
✓ 3 registro(s) encontrado(s):
  user:alice = Alice Silva
  user:bob = Bob Santos
  user:charlie = Charlie Costa

# Prefix search (faster for hierarchical keys)
lsm> SEARCH user: --prefix
✓ 3 registro(s) encontrado(s):
  user:alice = Alice Silva
  user:bob = Bob Santos
  user:charlie = Charlie Costa
```

**REST API Equivalent:** `GET /keys/search?q=query&prefix=true`

### 3. BATCH SET - Bulk Import from Files
**Command:** `BATCH SET <file>`

**Description:** Imports records from a text file in `key=value` format.

**File Format:**
```
# Comments start with #
user:alice=Alice Silva
user:bob=Bob Santos
product:1=Laptop Dell XPS 15
config:theme=dark
```

**Features:**
- Supports comments (lines starting with `#`)
- Skips empty lines
- Automatic key/value trimming
- Error reporting with line numbers
- Progress feedback

**Example:**
```bash
lsm> BATCH SET examples/batch_data.txt
Importando de examples/batch_data.txt...
✓ 23 registro(s) importado(s)
```

**REST API Equivalent:** `POST /keys/batch`

### 4. SCAN Improvement
**Command:** `SCAN <prefix>`

**Description:** Updated to use the optimized `search_prefix` method instead of showing "not implemented" warning.

**Example:**
```bash
lsm> SCAN user:
✓ 3 registro(s) com prefixo 'user:':
  user:alice = Alice Silva
  user:bob = Bob Santos
  user:charlie = Charlie Costa
```

## Improvements

### Enhanced Help System
Updated `print_help()` function to include all new commands with proper formatting:
```
STATS [ALL]               - Exibe estatísticas (básicas ou detalhadas)
SEARCH <query> [--prefix] - Busca registros (opcionalmente por prefixo)
BATCH SET <file>          - Importa registros de arquivo
```

### Enhanced DEMO Command
Updated demo to showcase all new features:
- Tests SEARCH in both modes
- Displays STATS ALL output
- Demonstrates prefix scanning

## Documentation

### New Files
- **`docs/CLI_GUIDE.md`**: Comprehensive CLI documentation
  - All command syntax and examples
  - Best practices
  - Troubleshooting guide
  - Complete workflow examples

- **`examples/batch_data.txt`**: Sample data file
  - Demonstrates file format for BATCH SET
  - Includes various data types (users, products, config)
  - Comments explaining usage

### Updated Files
- **`src/cli/mod.rs`**: Complete CLI implementation
- **`CHANGELOG_v1.5.md`**: This changelog

## Breaking Changes
None. All existing commands remain unchanged and backward compatible.

## Migration Guide
No migration needed. New commands are additive.

## Implementation Details

### Command Parsing
- Updated `splitn` to handle 4 parts for complex commands
- Maintains backward compatibility with existing command syntax

### Error Handling
- Graceful handling of file read errors
- Line-by-line error reporting for BATCH SET
- Validates file format and provides helpful error messages

### Performance
- SEARCH with `--prefix` flag uses optimized BTree iteration
- BATCH SET processes files line-by-line (memory efficient)
- STATS ALL uses existing engine methods (minimal overhead)

## Testing

### Manual Testing Completed
- [x] STATS ALL produces valid JSON
- [x] SEARCH works in both substring and prefix modes
- [x] BATCH SET imports data correctly
- [x] File format validation works properly
- [x] Error messages are helpful and accurate
- [x] SCAN command uses optimized search
- [x] All commands maintain backward compatibility

### Test Files
- `examples/batch_data.txt` - Sample data for testing

## CLI vs REST API Command Mapping

| CLI Command | REST API Endpoint | Status |
|-------------|------------------|--------|
| `SET <key> <value>` | `POST /keys` | ✅ Parity |
| `GET <key>` | `GET /keys/{key}` | ✅ Parity |
| `DELETE <key>` | `DELETE /keys/{key}` | ✅ Parity |
| `STATS` | `GET /stats` | ✅ Parity |
| `STATS ALL` | `GET /stats/all` | ✅ **New** |
| `SEARCH <query>` | `GET /keys/search?q=...` | ✅ **New** |
| `SEARCH <query> --prefix` | `GET /keys/search?q=...&prefix=true` | ✅ **New** |
| `BATCH SET <file>` | `POST /keys/batch` | ✅ **New** |
| `KEYS` | `GET /keys` | ✅ Parity |
| `ALL` | `GET /scan` | ✅ Parity |
| `COUNT` | N/A (CLI only) | ✅ CLI-only |
| `SCAN <prefix>` | `GET /scan?prefix=...` | ✅ Improved |

## Known Limitations

1. **Token Management**: CLI does not support token management commands (low priority - better suited for REST API)
2. **Feature Flags**: CLI does not yet support feature flag management (planned for future release)
3. **Large Files**: BATCH SET loads entire file into memory (acceptable for typical use cases)

## Future Enhancements (v1.6+)

### Phase 2: Feature Management (Medium Priority)
- [ ] `FEATURES` - List all feature flags
- [ ] `FEATURE SET <name> <value>` - Toggle feature flags

### Phase 3: Advanced Features (Low Priority)
- [ ] `EXPORT <file>` - Export data to file
- [ ] `IMPORT <file> [--format json|txt]` - Import with format detection
- [ ] `TOKEN` commands - CLI-based token management

## Contributors
- Elio Neto (@ElioNeto)

## Related Issues
- Closes #65 - Equalize CLI Commands with REST API Endpoints

## References
- [CLI Guide](docs/CLI_GUIDE.md)
- [REST API Documentation](docs/API.md)
- [Configuration Guide](docs/CONFIGURATION.md)

---

**Release Notes:** This release focuses on developer experience by providing CLI feature parity with the REST API. All Phase 1 (High Priority) commands have been implemented, tested, and documented.
