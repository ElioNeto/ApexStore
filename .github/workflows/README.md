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
**Trigger:** Pull Request opened/updated targeting `main`  
**Purpose:** Ensure code quality before merge  
**Steps:**
- Format check (`cargo fmt --check`)
- Linting (`cargo clippy -- -D warnings`)
- Tests (`cargo test --all-features`)
- Build (`cargo build --release`)

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
