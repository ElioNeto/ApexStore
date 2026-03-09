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
