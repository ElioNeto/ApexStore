# 🚀 Migration Guide: Gitflow → Trunk-Based Development

## 📌 Summary of Changes

ApexStore has migrated from **Gitflow** (with `develop`, `release/*`, `hotfix/*` branches) to **trunk-based development** with direct merges to `main`.

### Before (Gitflow)
```
feature/* → develop → release/* → main → manual tag + release
```

### After (Trunk-Based)
```
feature/* → main → auto version bump + tag + release
```

---

## ✅ What Changed

| Aspect | Old (Gitflow) | New (Trunk-Based) |
|--------|---------------|-------------------|
| **Main branch** | `main` (stable releases only) | `main` (always deployable) |
| **Development branch** | `develop` | ❌ Removed |
| **Feature branches** | `feature/*` → `develop` | `feat/*` / `fix/*` → `main` |
| **Release process** | Manual `release/*` branches | ✅ Automatic on merge |
| **Version bumping** | Manual in `Cargo.toml` | ✅ Auto-increment patch |
| **Tagging** | Manual `git tag vX.Y.Z` | ✅ Auto-created |
| **GitHub Release** | Manual creation | ✅ Auto-generated with notes |
| **CI checks** | Multiple workflows | 1 unified `pr-validation.yml` |

---

## 🛠️ New Workflow (Step-by-Step)

### 1. Creating a Feature

```bash
# Start from main (always up-to-date)
git checkout main
git pull origin main

# Create feature branch
git checkout -b feat/my-awesome-feature
# or
git checkout -b fix/critical-bug
```

### 2. Development & Commits

```bash
# Make changes
vim src/core/engine.rs

# Commit with conventional commits
git add .
git commit -m "feat: add caching layer to engine"
# or
git commit -m "fix: resolve memory leak in SSTable reader"

# Push to remote
git push origin feat/my-awesome-feature
```

### 3. Open Pull Request

1. Go to GitHub and open a PR from `feat/my-awesome-feature` → `main`
2. **CI automatically runs** (`pr-validation.yml`):
   - `cargo fmt --check`
   - `cargo clippy -- -D warnings`
   - `cargo test --all-features`
   - `cargo build --release`
3. Fix any CI failures
4. Request code review
5. Address review comments

### 4. Merge to Main

1. Once approved, click **"Merge Pull Request"**
2. **CI automatically runs** (`release.yml`):
   - Version bumps: `2.1.0` → `2.1.1`
   - Commits: `chore: bump version to 2.1.1 [skip ci]`
   - Creates tag: `v2.1.1`
   - Creates GitHub Release with changelog
3. **Done!** Your feature is released 🎉

---

## ⚠️ Breaking Changes & Cleanup

### Branches to Delete

After merging this migration PR, delete the following branches:

```bash
git push origin --delete develop      # No longer used
git push origin --delete release/*    # No longer used
```

### Local Cleanup

```bash
# Remove local references to deleted branches
git fetch --prune

# Delete local develop branch
git branch -D develop

# Set main as default tracking branch
git branch --set-upstream-to=origin/main main
```

---

## 📚 FAQ

### Q: What if I need a **minor** or **major** version bump?

**A:** The CI auto-increments **patch** by default. For minor/major:

```bash
# Option 1: Manually edit Cargo.toml in your PR
vim Cargo.toml
# Change: version = "2.1.0" → "2.2.0" (minor) or "3.0.0" (major)

# Option 2: Use cargo-bump (if installed)
cargo install cargo-bump
cargo bump minor  # or: cargo bump major
```

Then open PR as usual. The CI will detect the manually set version and **not** override it.

### Q: How do I create a **hotfix** for production?

**A:** Same as a feature:

```bash
git checkout main
git pull
git checkout -b fix/critical-security-issue
# ... make fix ...
git commit -m "fix: patch XSS vulnerability in API"
git push origin fix/critical-security-issue
# Open PR → main, merge, auto-release!
```

### Q: What if CI fails on my PR?

**A:** Fix the issues locally:

```bash
# Check format
cargo fmt

# Fix clippy warnings
cargo clippy --fix --allow-dirty

# Run tests
cargo test

# Commit fixes
git add .
git commit -m "chore: fix CI issues"
git push
```

CI will re-run automatically.

### Q: Can I still manually create releases?

**A:** Yes, but not recommended. The CI handles it better. If needed:

```bash
# Disable the release.yml workflow temporarily
# Then follow manual steps
```

### Q: What about `develop` branch history?

**A:** All commits from `develop` should be merged to `main` before deleting it:

```bash
# If develop has unmerged commits:
git checkout main
git merge develop
git push origin main

# Then delete develop
git push origin --delete develop
```

---

## 👥 Team Onboarding Checklist

- [ ] Read this migration guide
- [ ] Delete local `develop` branch
- [ ] Update Git remote tracking: `git fetch --prune`
- [ ] Read `.github/workflows/README.md`
- [ ] Test workflow: create a small PR, merge, verify auto-release
- [ ] Update any CI/CD documentation in wikis/Notion/etc.
- [ ] Notify team of new workflow in Slack/Discord

---

## 📞 Support

Questions? Open an issue labeled `workflow` or ping @ElioNeto.

---

**Migration Date:** March 9, 2026  
**Migrated By:** github-actions[bot] / Elio Neto

## SSTable recovery diagnostics

SSTable metadata is validated during discovery and snapshot restore. An invalid
Bloom filter is a performance degradation: the real key range and data remain
available, and reads proceed without the filter. Structural metadata failures
are quarantined instead of registering an empty table.

A missing encryption key or AES-GCM authentication failure aborts discovery or
snapshot preflight without moving any SSTables. Authentication failure can mean
a wrong key or damaged ciphertext; preserve the originals and verify the key
before attempting recovery. Plaintext `LSMSST03` files remain readable when an
encryption key is configured for `LSMSST04` files. Legacy zero-key files require the explicit offline migration below; automatic
recovery never rewrites them.

Quarantined files live in `sstables/quarantine/`. Each file has a sibling
`<name>.sst.quarantine-reason` JSON file with `original_filename`, `error`,
`engine_version`, and `timestamp_unix_ms`. Existing evidence is retained when a
filename is reused. Keep a backup of both files before repairing data.

`GET /stats`, `GET /stats/all`, and health diagnostics expose
`quarantined_sstables`, `unreadable_sstables`, and `degraded_bloom_sstables`.
These are instance-wide recovery counts, including previously quarantined
files found on restart. Readiness, startup, and comprehensive health return 503
while unreadable/quarantined files remain; liveness remains 200. A degraded
Bloom filter alone keeps readiness at 200. After restoring verified data,
archive the quarantined files and their reasons outside the database and
restart to recompute the recovery counters. The counters describe discovery
and snapshot recovery; they are not a full online integrity scan.

Metadata is bounded to 64 MiB before decompression. Files exceeding that limit
fail explicitly as unsupported inputs and are not automatically quarantined.
Snapshot SSTables are inspected before copying; conflicting live filenames,
invalid manifest paths, missing files and duplicate entries fail preflight.
Snapshot restore is still not a transactional replacement of a running database;
use a fresh destination and retain the source snapshot.

## Encrypted WAL recovery failures

Recovery of a checksum-valid encrypted V3 WAL frame now requires an enabled key.
A missing key, AES-GCM authentication failure, or malformed authenticated record
aborts recovery without returning partially replayed records. This applies to
both the default WAL and each column-family WAL. Recovery preserves the original
WAL bytes, so an operator can correct the key and retry. An authentication error
can also indicate damaged ciphertext: retain a backup before investigating.

Plaintext V0/V1/V2 frames retain their compatibility behavior, and mixed V2/V3
logs remain readable with the correct key. Existing handling of physical CRC
failures and partial frames is unchanged. Use the explicit offline migration below for legacy zero-key logs.

## Compaction encryption configuration

Built-in compaction strategies now use `EngineOptions.encryption`, matching
flush and point reads. They do not reload a key file from the copied
`StorageConfig` or substitute a zero key when its key path is absent. A running
engine retains its loaded key even if the key file is subsequently unavailable;
restarting still requires restoring the correct key configuration.

When encryption is disabled, compacted SSTables retain the plaintext `LSMSST03`
format. With a nonzero key, output remains `LSMSST04` under that same key. This
also applies when invoking a built-in strategy directly: the runtime options
are authoritative for encryption. Existing zero-key outputs must be migrated explicitly before reopening with
the new defaults.

## Upgrade: explicit encryption and legacy zero-key migration

Encryption is now **disabled by default**, consistently for programmatic and
serialized application configuration. Set `encryption_enabled = true` only with
a valid nonzero 32-byte key file (64 hexadecimal characters). Missing, unreadable,
malformed and all-zero key files fail construction before the engine opens its
storage. The server accepts `ENCRYPTION_ENABLED=true` and
`ENCRYPTION_KEY_PATH=/run/secrets/apexstore.key`; invalid booleans fail startup.
The CLI enables encryption when `--encrypt-key-file` is supplied.

This is a compatibility change. Older defaults wrote `LSMSST04` files and V3 WAL
frames with a publicly known all-zero key. Startup diagnoses that format and
names `migrate-encryption`; it does not silently use the legacy key or quarantine
those files. Retain the originals and any backups while migrating.

Stop all servers and CLI processes using the source database. Store keys outside
the database directory. Choose an output directory that does not exist, under an
existing parent, outside the source. To migrate legacy zero-key data:

```bash
apexstore-cli --db /data/old migrate-encryption --legacy-zero-key \
  --output /data/migrated --target-key-file /run/secrets/apexstore.key
```

To explicitly remove encryption instead:

```bash
apexstore-cli --db /data/old migrate-encryption --legacy-zero-key \
  --output /data/plaintext --plaintext
```

For rotation from a real key, replace `--legacy-zero-key` with
`--source-key-file /run/secrets/old.key`. Omit both source options for a plaintext
source. The target must always be selected explicitly: `--target-key-file` or
`--plaintext`. Migration never generates or prints a key.

The command acquires the source database lock and refuses live nested databases.
It rewrites SSTables and every WAL frame into a private staging directory,
retaining record timestamps, expiration, tombstones, range tombstones, column
family metadata and frame ordering. Existing manifests and ancillary files are
copied without rewriting their contents. Snapshots under the source tree are
also converted. Bloom filters are rebuilt from the retained records. The command
does not reconstruct missing manifests or repair corrupted records.

Any checksum, decoding, authentication or record-count failure aborts migration;
partial WAL frames are rejected rather than discarded. Symlinks and special
files are refused. Failed conversion removes its staging directory and leaves
the original data intact. Successful conversion publishes the staged directory
with an OS-enforced atomic rename without replacement and reports SSTable/frame/file
counts as JSON. The migration command currently requires Linux/Android, Apple or
Redox with filesystem support for this operation; other platforms fail safely.
Existing output directories are refused. The original database is retained, including its old
ciphertext, and should be archived according to your data-retention policy.

Verify the destination before switching applications to it:

```bash
apexstore-cli --db /data/migrated --encrypt-key-file /run/secrets/apexstore.key stats
# For a plaintext destination, omit --encrypt-key-file.
```

Update the application's data directory and key configuration together. Keep the
key available for future restarts; a running engine retains the loaded key but a
restart does not recover it from its SSTables or WAL.

Library compatibility changes: replace `EngineOptions::from(&config)` and the
infallible `from_config` with `EngineOptions::try_from_config(&config)?` or
`EngineOptions::try_from(&config)?`. The standalone `SstableBuilder::new` and
`SstableReader::open` constructors also validate the supplied encryption settings
before accessing the table. `EncryptionConfig` no longer implements serde
serialization/deserialization; its Debug is redacted and its key is zeroized on
drop. Serialized layout-only `storage::config::StorageConfig` omits runtime
encryption and rejects an embedded encryption field. Use the application's
`infra::config::StorageConfig` key-path settings to reload keys explicitly.

The scrubber accepts a loaded key through `scrub_file_with_encryption` or
`DataScrubber::new_with_encryption`. Without the necessary key, `skipped = true`
and `valid = false` distinguish an unverified file from corruption; no file is
moved. With the key it verifies both CRC32 and encrypted block authentication.

These constructor/serde changes require a semver review before crate publication;
they must not be published as an automatic patch release.
