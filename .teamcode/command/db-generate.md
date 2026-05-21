---
description: "Manage database schema and storage engine internals"
---

Work with the ApexStore storage engine schema and internals.

ApexStore is an LSM-tree key-value store. It does not use traditional SQL migrations.
Schema is managed via Column Families.

## Useful commands

### Start a fresh database:
```bash
rm -rf .apexstore/ && cargo run --bin apexstore-server
```

### Inspect SSTable files:
```bash
ls -la .apexstore/
xxd .apexstore/*.sst | head -50
```

### Run storage engine tests:
```bash
cargo test --test integration_test
```

$ARGUMENTS
