# ApexStore CLI Guide

## Overview

The ApexStore CLI provides an interactive REPL (Read-Eval-Print Loop) interface for managing your LSM-Tree key-value store. This guide covers all available commands and their usage.

## Starting the CLI

```bash
cargo run --release
```

Or if you've built the binary:

```bash
./target/release/apexstore
```

## Basic Commands

### SET - Insert or Update Data

Inserts or updates a key-value pair.

**Syntax:**
```
SET <key> <value>
```

**Examples:**
```bash
lsm> SET user:alice Alice Silva
✓ SET 'user:alice' executado com sucesso

lsm> SET product:1 Laptop Dell XPS 15
✓ SET 'product:1' executado com sucesso
```

### GET - Retrieve Data

Retrieves the value for a given key.

**Syntax:**
```
GET <key>
```

**Examples:**
```bash
lsm> GET user:alice
✓ 'user:alice' = 'Alice Silva'

lsm> GET nonexistent
⚠ Chave 'nonexistent' não encontrada
```

### DELETE - Remove Data

Deletes a key by creating a tombstone marker.

**Syntax:**
```
DELETE <key>
```

**Aliases:** `DEL`

**Examples:**
```bash
lsm> DELETE user:alice
✓ DELETE 'user:alice' executado (tombstone criado)

lsm> DEL product:1
✓ DELETE 'product:1' executado (tombstone criado)
```

## Search Commands

### SEARCH - Advanced Search

Searches for records matching a query. Supports both substring and prefix modes.

**Syntax:**
```
SEARCH <query> [--prefix]
```

**Substring Search** (default):
```bash
lsm> SEARCH user
✓ 3 registro(s) encontrado(s):

  user:alice = Alice Silva
  user:bob = Bob Santos
  user:charlie = Charlie Costa
```

**Prefix Search** (faster for hierarchical keys):
```bash
lsm> SEARCH user: --prefix
✓ 3 registro(s) encontrado(s):

  user:alice = Alice Silva
  user:bob = Bob Santos  
  user:charlie = Charlie Costa
```

**Use Cases:**
- `SEARCH user` - Find all keys containing "user" (substring)
- `SEARCH user: --prefix` - Find all keys starting with "user:" (prefix)
- `SEARCH :1` - Find all keys containing ":1"
- `SEARCH product: --prefix` - Find all product keys

### SCAN - Prefix Scan

Lists all records with a specific prefix (equivalent to `SEARCH <prefix> --prefix`).

**Syntax:**
```
SCAN <prefix>
```

**Examples:**
```bash
lsm> SCAN user:
✓ 3 registro(s) com prefixo 'user:':

  user:alice = Alice Silva
  user:bob = Bob Santos
  user:charlie = Charlie Costa

lsm> SCAN config:
✓ 4 registro(s) com prefixo 'config:':

  config:theme = dark
  config:language = pt-BR
  config:notifications = enabled
  config:auto_save = true
```

## Listing Commands

### ALL - List All Records

Displays all records in the database in a formatted table.

**Syntax:**
```
ALL
```

**Example:**
```bash
lsm> ALL
Listando todos os registros...

┌─────────────────────────────────────────────────┐
│  Chave                │  Valor                 │
├─────────────────────────────────────────────────┤
│  user:alice           │  Alice Silva           │
│  user:bob             │  Bob Santos            │
│  product:1            │  Laptop Dell XPS 15    │
└─────────────────────────────────────────────────┘
```

### KEYS - List All Keys

Lists only the keys (without values).

**Syntax:**
```
KEYS
```

**Example:**
```bash
lsm> KEYS
Total de chaves: 5

  1. user:alice
  2. user:bob
  3. user:charlie
  4. product:1
  5. config:theme
```

### COUNT - Count Records

Counts the number of active records.

**Syntax:**
```
COUNT
```

**Example:**
```bash
lsm> COUNT
✓ Total de registros ativos: 5
```

## Statistics Commands

### STATS - Basic Statistics

Displays basic engine statistics.

**Syntax:**
```
STATS
```

**Example:**
```bash
lsm> STATS
LSM Stats:
 MemTable: 3 records, ~2 KB
 SSTables: 2 files
 Cache: 12/256 blocks
```

### STATS ALL - Detailed Statistics

Displays comprehensive statistics in JSON format including memory, disk, cache, and WAL metrics.

**Syntax:**
```
STATS ALL
```

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

**Fields Explained:**
- `mem_records` - Number of records in MemTable
- `mem_kb` - MemTable size in KB
- `sst_files` - Number of SSTable files on disk
- `sst_records` - Total records across all SSTables
- `sst_kb` - Total SSTable disk usage in KB
- `wal_kb` - Write-Ahead Log size in KB
- `total_records` - Sum of MemTable + SSTable records
- `memtable_max_size` - Configured MemTable flush threshold in KB

## Batch Operations

### BATCH - Generate Test Data

Inserts N test records for benchmarking.

**Syntax:**
```
BATCH <count>
```

**Example:**
```bash
lsm> BATCH 1000
Inserindo 1000 registros...
✓ 1000 registros inseridos em 45.23ms
  Taxa: 22104 ops/s
```

### BATCH SET - Bulk Import from File

Imports records from a text file.

**Syntax:**
```
BATCH SET <file>
```

**File Format:**
```
# Lines starting with # are comments
key1=value1
key2=value2
user:alice=Alice Silva
product:1=Laptop
```

**Example:**
```bash
lsm> BATCH SET examples/batch_data.txt
Importando de examples/batch_data.txt...
✓ 23 registro(s) importado(s)
```

**File Format Rules:**
- Each line: `key=value`
- Lines starting with `#` are ignored (comments)
- Empty lines are skipped
- Keys and values are automatically trimmed
- No quotes needed around keys or values

**Sample File (`data.txt`):**
```
# User data
user:alice=Alice Silva
user:bob=Bob Santos

# Product data  
product:1=Laptop Dell XPS 15
product:2=Mouse Logitech MX Master

# Configuration
config:theme=dark
config:language=pt-BR
```

**Use Cases:**
- Initial data seeding
- Migrating data from other systems
- Testing with realistic datasets
- Configuration management

## Utility Commands

### DEMO - Run Demonstration

Executes an automated demo showcasing engine features.

**Syntax:**
```
DEMO
```

**What it does:**
1. Inserts sample users
2. Reads and displays data
3. Updates a record
4. Deletes a record
5. Forces MemTable flush
6. Tests search commands
7. Displays statistics

### CLEAR - Clear Screen

Clears the terminal screen.

**Syntax:**
```
CLEAR
```

### HELP - Show Help

Displays available commands.

**Syntax:**
```
HELP
```

**Alias:** `?`

### EXIT - Exit CLI

Exits the REPL.

**Syntax:**
```
EXIT
```

**Aliases:** `QUIT`, `Q`

## Command Comparison: CLI vs REST API

All CLI commands have equivalent REST API endpoints:

| CLI Command | REST API Endpoint | Method |
|-------------|------------------|--------|
| `SET key value` | `/keys` | POST |
| `GET key` | `/keys/{key}` | GET |
| `DELETE key` | `/keys/{key}` | DELETE |
| `SEARCH query --prefix` | `/keys/search?q=query&prefix=true` | GET |
| `STATS` | `/stats` | GET |
| `STATS ALL` | `/stats/all` | GET |
| `BATCH SET file` | `/keys/batch` | POST |
| `KEYS` | `/keys` | GET |
| `SCAN prefix` | `/scan?prefix=...` | GET |

## Best Practices

### Naming Conventions

Use hierarchical keys with colons for organization:
```
user:123:profile
user:123:settings
product:456:name
product:456:price
session:abc:token
```

### Efficient Searches

- Use `--prefix` for hierarchical keys (faster)
- Without `--prefix`: slower substring search
- Prefix searches leverage BTree ordering

### Performance Tips

1. **Use BATCH for bulk inserts** (faster than individual SETs)
2. **Monitor with STATS ALL** to track memory usage
3. **Use prefix-based keys** for efficient SCAN operations
4. **Avoid very long keys** (increases memory overhead)

### Data Management

1. **Backup data files** before major operations
2. **Use BATCH SET** for reproducible setups
3. **Monitor MemTable size** with STATS
4. **Plan key namespaces** before loading data

## Troubleshooting

### Common Issues

**Q: "Comando desconhecido" error**
- A: Check spelling, commands are case-insensitive
- Use `HELP` to see available commands

**Q: BATCH SET fails to read file**
- A: Check file path (relative to CLI working directory)
- Verify file permissions
- Ensure file format is correct (key=value)

**Q: MemTable keeps flushing**
- A: Normal behavior when reaching size limit
- Check `memtable_max_size` configuration
- Use `STATS` to monitor

**Q: Slow search performance**
- A: Use `--prefix` flag for hierarchical keys
- Consider adding indexes in future versions

## Examples

### Complete Workflow Example

```bash
# Start CLI
cargo run --release

# Import initial data
lsm> BATCH SET data/users.txt
✓ 100 registro(s) importado(s)

# Verify import
lsm> COUNT
✓ Total de registros ativos: 100

# Search for specific users
lsm> SEARCH admin --prefix
✓ 5 registro(s) encontrado(s):
  admin:root = Root User
  admin:alice = Alice Admin
  ...

# Check statistics
lsm> STATS ALL
{
  "mem_records": 100,
  "sst_files": 0,
  ...
}

# Update a record
lsm> SET admin:root Super Admin
✓ SET 'admin:root' executado com sucesso

# Delete old records
lsm> DELETE user:temp
✓ DELETE 'user:temp' executado

# Final stats
lsm> STATS
LSM Stats:
 MemTable: 99 records, ~15 KB
 SSTables: 0 files

# Exit
lsm> exit
👋 Encerrando LSM-Tree CLI...
```

## Related Documentation

- [Configuration Guide](./CONFIGURATION.md) - Engine configuration options
- [API Reference](./API.md) - REST API endpoints
- [Contributing Guide](./CONTRIBUTING.md) - Development guidelines
- [README](../README.md) - Project overview

---

**Need help?** Open an issue on [GitHub](https://github.com/ElioNeto/ApexStore/issues)
