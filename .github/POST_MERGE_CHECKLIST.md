# ✅ Post-Merge Checklist

**Branch:** `chore/ci-cd-pipeline`  
**Target:** `main`  
**Purpose:** Migrate from Gitflow to Trunk-Based Development  

---

## 🔴 CRITICAL - Do Immediately After Merge

### 1. Verify Auto-Release Triggered

- [ ] Go to [Actions](https://github.com/ElioNeto/ApexStore/actions/workflows/release.yml)
- [ ] Check that `Auto Release` workflow started
- [ ] Wait for completion (~2-3 minutes)
- [ ] Verify new version in [Releases](https://github.com/ElioNeto/ApexStore/releases) (should be v2.1.1)

**Expected:**
```
✅ Auto Release workflow completed
✅ Cargo.toml updated to 2.1.1
✅ Tag v2.1.1 created
✅ GitHub Release v2.1.1 published
```

### 2. Delete Legacy Branches

```bash
# Delete 'develop' branch (no longer used)
git push origin --delete develop

# Optional: Delete any stale release/* branches
git push origin --delete release/v1.7.0
# (repeat for other release/* branches if they exist)
```

### 3. Update Local Repository

```bash
# Fetch latest changes
git fetch --prune

# Switch to main
git checkout main
git pull origin main

# Delete local develop branch
git branch -D develop

# Verify you're on latest
git log --oneline -5
# Should show: "chore: bump version to 2.1.1 [skip ci]"
```

---

## 🟡 IMPORTANT - Do Within 24 Hours

### 4. Verify CI/CD Works End-to-End

**Create a test PR:**

```bash
# Create test feature
git checkout -b test/ci-cd-validation
echo "# CI/CD Test" >> TEST.md
git add TEST.md
git commit -m "test: validate new CI/CD pipeline"
git push origin test/ci-cd-validation
```

- [ ] Open PR to `main` on GitHub
- [ ] Verify `PR Validation` workflow runs
- [ ] Check that fmt/clippy/test/build all pass
- [ ] Merge the PR
- [ ] Verify `Auto Release` creates v2.1.2
- [ ] Delete test branch: `git push origin --delete test/ci-cd-validation`

### 5. Update Team Documentation

- [ ] Update internal wiki/Notion with new workflow
- [ ] Update onboarding docs to reference `MIGRATION_GUIDE.md`
- [ ] Remove references to `develop` branch from docs
- [ ] Update any CI/CD diagrams

### 6. Notify Team

**Send announcement in Slack/Discord/Email:**

```markdown
🚀 **ApexStore CI/CD Migration Complete!**

We've migrated from Gitflow to Trunk-Based Development.

**What changed:**
- All PRs now go to `main` (no more `develop`)
- Auto version bump on every merge
- Auto GitHub releases

**Action required:**
1. Read: https://github.com/ElioNeto/ApexStore/blob/main/MIGRATION_GUIDE.md
2. Delete local `develop` branch: `git branch -D develop`
3. Update your workflow: feature → PR to main → merge → auto-release

**Questions?** Ping @ElioNeto or open an issue with label `workflow`
```

---

## 🟢 OPTIONAL - Nice to Have

### 7. Update Branch Protection Rules

Go to [Settings → Branches](https://github.com/ElioNeto/ApexStore/settings/branches):

- [ ] Ensure `main` has protection enabled
- [ ] Require status checks: `validate / Validate PR`
- [ ] Require PR reviews: 1 approver minimum
- [ ] Dismiss stale reviews on new commits
- [ ] Require linear history (optional)
- [ ] Delete `develop` branch protection (if exists)

### 8. Configure Repository Settings

Go to [Settings → General](https://github.com/ElioNeto/ApexStore/settings):

- [ ] Default branch: Set to `main` (should already be)
- [ ] Allow squash merging: ✅ Enabled
- [ ] Allow merge commits: ❌ Disabled (optional)
- [ ] Allow rebase merging: ❌ Disabled (optional)
- [ ] Automatically delete head branches: ✅ Enabled

### 9. Update CHANGELOG

Add entry to `CHANGELOG.md` (if exists):

```markdown
## [2.1.1] - 2026-03-09

### Changed
- Migrated CI/CD from Gitflow to Trunk-Based Development
- Automated version bumping on merge to main
- Automated GitHub releases with auto-generated notes
- Removed legacy workflows: develop-to-release, feature-fix, release-workflow, test
- Added new workflows: pr-validation, release

### Documentation
- Added MIGRATION_GUIDE.md for team onboarding
- Added .github/workflows/README.md for workflow documentation
- Added .github/PULL_REQUEST_TEMPLATE.md for PR standardization
- Updated README.md with CI/CD section and badges
```

### 10. Monitor First Week

**Track metrics for 7 days:**

- [ ] Number of PRs opened to `main`
- [ ] CI success rate (should be >95%)
- [ ] Average time from PR open to merge
- [ ] Number of releases created (should match # of merges)
- [ ] Team feedback on new workflow

**Log issues:**
- Any workflow failures
- Version bump problems
- Release creation errors
- Developer confusion

---

## 🚨 Rollback Plan (If Needed)

**If critical issues arise:**

1. **Disable auto-release workflow:**
   ```bash
   # Create rollback PR
   git checkout -b hotfix/disable-auto-release
   # Edit .github/workflows/release.yml, comment out all steps
   git commit -m "hotfix: disable auto-release temporarily"
   git push
   # Merge immediately
   ```

2. **Restore old workflows:**
   ```bash
   git checkout main
   git revert <commit-sha-that-removed-old-workflows>
   git push
   ```

3. **Recreate `develop` branch:**
   ```bash
   git checkout -b develop
   git push origin develop
   ```

---

## 📊 Success Metrics

**After 1 week, you should see:**

- ✅ 100% of PRs targeting `main` (not `develop`)
- ✅ Auto-releases matching number of merges
- ✅ Reduced time to production (no manual release steps)
- ✅ Zero manual version bumps in Cargo.toml
- ✅ Team comfortable with new workflow

---

## ❓ FAQ

**Q: The auto-release didn't trigger, what do I do?**  
A: Check the workflow logs. Common causes:
- Commit message contains `[skip ci]`
- Actor is `github-actions[bot]` (anti-loop)
- Permissions issue (verify `contents: write`)

**Q: Can I manually create a release?**  
A: Yes, but not recommended. Use the UI or `gh release create`.

**Q: How do I do a major/minor version bump?**  
A: Manually edit `Cargo.toml` version in your PR before merging.

**Q: What if I need to hotfix production urgently?**  
A: Same workflow! Create `fix/critical-bug` branch, open PR to `main`, merge. Auto-release handles the rest.

---

**✅ All tasks complete?** Mark this checklist as done and archive this file.

**Date Completed:** _____________  
**Completed By:** _____________  
**Notes/Issues:** _____________  
