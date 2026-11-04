# ApexStore CI/CD Workflows

## 🎯 Overview

This repository uses a **trunk-based development** workflow with automated releases. All features are developed in short-lived branches and merged directly into `main`.

## 📦 Workflow Architecture

```
feature/xyz branch
       ↓
   (open PR to main)
       ↓
  ✅ PR Validation (pr-validation.yml)
     - cargo fmt --check
     - cargo clippy
     - cargo test
     - cargo build --release
       ↓
   (merge to main)
       ↓
  🚀 Auto Release (release.yml)
     - Bump Cargo.toml version (patch)
     - Commit version change
     - Create git tag (v2.1.X)
     - Create GitHub Release
```

## 📝 Active Workflows

### 1. `pr-validation.yml`
**Trigger:** Pull requests targeting `main`, or `workflow_call` from CI and release
**Purpose:** Run the same required checks before merge and publication
**Steps:**
- Format check (`cargo fmt --check`)
- Linting (`cargo clippy -- -D warnings`)
- Tests (`cargo test --all-features`)
- Build (`cargo build --locked --release`)
- Security audit (failures block publication)
- CI script regression tests (Python and Node.js)

### 2. `release.yml`
**Trigger:** Push to `main` (i.e., PR merged)  
**Purpose:** Automatic versioning and release creation  
**Steps:**
1. Read current version from `Cargo.toml`
2. Increment **patch** version (e.g., `2.1.0` → `2.1.1`)
3. Update `Cargo.toml` and `Cargo.lock`
4. Commit with message: `chore: bump version to X.Y.Z [skip ci]`
5. Create git tag `vX.Y.Z`
6. Create GitHub Release with auto-generated notes

**Anti-loop protection:** Skips execution if actor is `github-actions[bot]`

### 3. `deploy-docs.yml`
**Trigger:** Push to `main`  
**Purpose:** Deploy Rustdoc to GitHub Pages  
*(unchanged from previous workflow)*

## 🔄 Development Workflow

### Creating a feature

```bash
# 1. Create feature branch from main
git checkout main
git pull origin main
git checkout -b feat/awesome-feature

# 2. Develop and commit
git add .
git commit -m "feat: add awesome feature"
git push origin feat/awesome-feature

# 3. Open PR to 'main' on GitHub
# - pr-validation.yml runs automatically
# - Fix any CI failures
# - Get code review approval

# 4. Merge PR
# - release.yml triggers automatically
# - Version bumps from 2.1.0 → 2.1.1
# - Tag v2.1.1 created
# - Release v2.1.1 published
```

## ⚠️ Important Notes

### Version Bumping Strategy
- **Patch bump** (default): All merges to `main` increment the patch version
- For **minor/major** bumps: manually edit `Cargo.toml` before opening the PR

### Skipping CI
Commits with `[skip ci]` in the message won't trigger workflows (used by the bot to avoid loops)

### Permissions
The `release.yml` workflow requires `contents: write` permission (already configured)

## 🛠️ Removed Workflows

The following Gitflow-based workflows were removed:
- `develop-to-release.yml`
- `feature-fix-workflow.yml`
- `release-workflow.yml`
- `test.yml`

They are replaced by the simpler trunk-based approach above.

## 📚 Resources

- [Trunk-Based Development](https://trunkbaseddevelopment.com/)
- [GitHub Actions - Workflow syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Semantic Versioning](https://semver.org/)

## Validation guarantees

`ci.yml` calls `pr-validation.yml` on pushes to main. `release.yml` also
requires that reusable validation to succeed before the release job starts.
The benchmark workflow propagates benchmark failures and retains diagnostic
artifacts. Its README formatter rejects empty or malformed measurements; it
does not install or invoke an external AI formatter.

The local `workflow-agent.ts` runs only `run:` steps via Docker. It cannot
execute GitHub Actions (`uses:`) or reusable workflows, and jobs with no
runnable commands now fail instead of reporting success. A local script run
is not a substitute for the hosted workflow.

Run the script regression tests from the repository root:

```bash
npm ci --prefix scripts
python3 -m unittest discover -s scripts -p 'test_*.py'
node --import ./scripts/node_modules/tsx/dist/loader.mjs --test scripts/workflow-agent.test.ts
```

The compaction stress data can be replayed with:

```bash
APEXSTORE_STRESS_SEED=430 cargo test --locked --all-features --test randomized_competitive test_flush_compaction_stress -- --exact --nocapture
```

The seed fixes the generated operations; background-thread scheduling remains
concurrent. A separate deterministic unit regression covers the multi-bucket
replacement defect without depending on thread timing.

## Temporary HTTP/2 dependency integration

The application enables `actix-web/http2-native` from upstream
[Actix PR #4321](https://github.com/actix/actix-web/pull/4321), pinned to revision
`4537c9f5b407c6f7c0e8037b87b8bebbf6388d80`. This PR is not yet merged or released.
Both Actix crates are patched to the same revision. The GraphQL adapter's
legacy `http2` feature is disabled because it forwards to Actix's old
compatibility feature; HTTP/2 itself remains enabled through the native backend.
Other previously enabled Actix features remain explicit in Cargo.toml.

`cargo tree --all-features -i h2` must resolve only h2 0.4.20 or newer.
`tests/http2.rs` checks real HTTP/2 key/value requests and sends raw empty DATA
frames against a server holding an unread body. The connection must terminate
with GOAWAY/ENHANCE_YOUR_CALM. TLS tests independently verify HTTP/1.1 and HTTP/2
with a trusted test certificate.

Cargo `[patch]` overrides are not inherited by downstream library consumers.
Consequently `check_release_dependencies.py` deliberately blocks publication
before any version bump or tag while these overrides exist. Remove the patches
only after native HTTP/2 is available in a published Actix version, update the
normal dependency constraints and lockfile, and rerun tests and audit. Do not
remove the release guard to publish an untested dependency graph.
