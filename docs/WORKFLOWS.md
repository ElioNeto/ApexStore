# 🔄 GitHub Workflows Documentation

ApexStore uses automated GitHub Actions workflows to manage the development lifecycle, from features to releases.

## 📊 Overview

```
feature/fix branches → develop → release/vX.Y.Z → main
       │                │            │              │
       └── Auto PR    └─ Auto PR  └─ Issues*   └─ Tag + Release
          + comments*     created      closed*
          on issues*                   

* = Optional, only when issues are referenced
```

## 🛠️ Available Workflows

### 1. Feature/Fix Workflow

**File**: `.github/workflows/feature-fix-workflow.yml`

**Trigger**: Push to `feature/**` or `fix/**` branches

#### What it does:

1. **Build & Test**
   - Compiles project: `cargo build --release --all-features`
   - Runs tests: `cargo test --all-features`
   - Checks linting: `cargo clippy --all-features -- -D warnings`

2. **Creates PR to develop**
   - Automatically detects if there are new commits
   - Creates PR to `develop` with:
     - List of referenced issues *(if any)*
     - Commit summary
     - Test status
   - Doesn't duplicate existing PRs

3. **Comments on Issues** *(optional)*
   - **Only runs if issues are referenced**
   - Identifies issues mentioned in commits
   - Adds **stacked** comments with updates:
     - Recent commits
     - Branch link
     - Development status

#### How to use:

**With issues:**
```bash
git checkout -b feature/my-feature
git commit -m "feat: implement X (#123)"
git commit -m "fix: resolve Y (fixes #124)"
git push origin feature/my-feature
```

**Without issues (also works!):**
```bash
git checkout -b feature/refactoring
git commit -m "refactor: improve code structure"
git commit -m "chore: update dependencies"
git push origin feature/refactoring
# ✅ PR created normally, without issues section
```

---

### 2. Develop to Release Workflow

**File**: `.github/workflows/develop-to-release.yml`

**Triggers**:
- Push to `develop` → Creates/updates release PR
- PR merged to `main` → Closes issues automatically *(if any)*

#### What it does:

**On push to develop:**

1. **Determines version**
   - Analyzes commits since last tag
   - Calculates bump (major/minor/patch):
     - `BREAKING CHANGE:`, `feat!:`, `fix!:` → Major
     - `feat:` → Minor
     - Others → Patch

2. **Creates release branch**
   - `release/vX.Y.Z`
   - Syncs with `develop`

3. **Creates PR to main**
   - Title: `🚀 Release vX.Y.Z`
   - Draft mode (requires approval)
   - Contains:
     - Configurable release type (alpha/beta/lts)
     - List of resolved issues *(if any)*
     - Complete changelog
     - Validation checklist

**On release PR merge:**

4. **Closes Issues Automatically** *(optional)*
   - **Only runs if issues are referenced**
   - Extracts referenced issues from commits
   - Adds final comment:
     ```
     ✅ Resolved in Release vX.Y.Z
     
     This issue has been fixed and released.
     Release: [View Release](link)
     ```
   - Closes issue with "completed" reason
   - **If no issues**: workflow completes normally without errors

---

## 🏷️ Issue Reference (Optional)

### When to Use Issues

✅ **Use when:**
- Fixing a reported bug
- Implementing a requested feature
- Want automatic traceability
- Want automatic notifications

⚪ **Don't need to use when:**
- Internal refactoring
- Dependency updates
- Performance improvements without issue
- Documentation
- Chores and minor tasks

### Supported Syntax:

```bash
# Any of these forms are detected:
git commit -m "feat: add feature (#123)"
git commit -m "fix: resolve bug (fixes #124)"
git commit -m "refactor: improve code (closes #125)"
git commit -m "docs: update (resolved #126)"
```

### Recognized Keywords:

- `close`, `closes`, `closed`
- `fix`, `fixes`, `fixed`
- `resolve`, `resolves`, `resolved`
- Simple: `#123`

---

## 📋 Flow Examples

### Example 1: With Issues

```bash
# Issue: #31 - Implement Bearer Token Authentication

git checkout -b feature/bearer-auth
git commit -m "feat: add auth module (#31)"
git commit -m "feat: add auth config (#31)"
git push origin feature/bearer-auth

# ✅ Workflow runs:
#    - Build + Tests pass
#    - PR created: feature/bearer-auth → develop
#    - Comment added to #31
#    - Issue listed in PR
```

### Example 2: Without Issues

```bash
# General refactoring - no specific issue

git checkout -b refactor/improve-performance
git commit -m "refactor: optimize database queries"
git commit -m "perf: add caching layer"
git push origin refactor/improve-performance

# ✅ Workflow runs:
#    - Build + Tests pass
#    - PR created: refactor/improve-performance → develop
#    - No issues section (normal!)
#    - Changelog shows commits normally
```

### Example 3: Release with Mix

```bash
# Merge to develop (some commits with issues, others without)

git checkout develop
git merge feature/bearer-auth  # has issue #31
git merge refactor/performance  # no issue
git push origin develop

# ✅ Workflow runs:
#    - Calculates version: v2.1.0 → v2.2.0
#    - Creates branch: release/v2.2.0
#    - Creates PR: release/v2.2.0 → main
#    - Lists only issue #31 (which was referenced)
#    - Changelog shows ALL commits

# When merging release PR:
# ✅ Issue #31 closed automatically
# ✅ Commits without issue ignored (no error)
```

---

## 🔍 Workflow Behavior

### Feature/Fix Workflow

| Situation | Behavior |
|----------|----------|
| Commits with issues | PR created + issues listed + comments on issues |
| Commits without issues | PR created + "No issues referenced" |
| Mix | PR created + only found issues listed |
| Non-existent issues | Ignores and continues (no error) |
| Already closed issues | Doesn't comment (silent skip) |

### Develop to Release Workflow

| Situation | Behavior |
|----------|----------|
| Commits with issues | PR lists issues + on merge closes automatically |
| Commits without issues | PR without issues section + on merge completes normally |
| Mix | PR lists only found issues |
| Non-existent issues | Ignores and continues (log warning) |
| Already closed issues | Tries to close but ignores error |

---

## ⚙️ Logs and Debugging

### Normal Messages (not errors)

```
ℹ️ No issues referenced in commits - skipping
```
**Meaning**: No issue was mentioned. Normal for commits without tracking.

```
⏭️ Skipping issue #123 (state: CLOSED)
```
**Meaning**: Issue was already closed. Workflow automatically skips.

```
⚠️ Issue #999 not found - skipping
```
**Meaning**: Issue doesn't exist. May be typo in commit, workflow continues.

---

## 📚 Best Practices

### When to Reference Issues

✅ **Recommended**:
```bash
# Bug fixes
git commit -m "fix: resolve authentication bug (fixes #54)"

# Requested features
git commit -m "feat: add JWT support (#31)"

# Specific improvements
git commit -m "perf: optimize query (closes #67)"
```

### When NOT to Reference

✅ **Also acceptable**:
```bash
# Internal refactoring
git commit -m "refactor: restructure auth module"

# Dependency updates
git commit -m "chore: update dependencies"

# Documentation
git commit -m "docs: add API examples"

# Small fixes
git commit -m "style: fix formatting"
```

---

## ⚠️ Troubleshooting

### "Workflow didn't comment on issue"

**Possible causes**:
1. ✅ **Normal**: Issue wasn't referenced in commit
2. ✅ **Normal**: Issue was already closed
3. ⚠️ **Check**: Is issue number correct?
4. ⚠️ **Check**: Is reference syntax correct?

### "Issue didn't close after release"

**Possible causes**:
1. ✅ **Normal**: Issue wasn't referenced in any commit
2. ✅ **Normal**: Issue was already closed
3. ⚠️ **Check**: Was PR merged (not just closed)?
4. ⚠️ **Check**: Did branch follow `release/*` pattern?

### "Workflow failed"

**Checklist**:
- [ ] Build passed locally?
- [ ] Tests passed?
- [ ] Clippy without errors?
- [ ] GitHub Actions permissions enabled?

---

## 🎯 Summary

### TL;DR

- ✅ **Issues are OPTIONAL** - workflows work with or without
- ✅ **Use issues for traceability** - automatic closing is bonus
- ✅ **Without issues is valid** - for refactoring, chores, etc
- ✅ **Mix is accepted** - some commits with, others without issues
- ✅ **Workflows are resilient** - don't break due to lack of issues

### Minimum Flow (without issues)

```bash
1. feature/x → develop
   ✅ Build + Test + PR created

2. develop → release/vX.Y.Z → main
   ✅ Version + Tag + Changelog

No issue needed!
```

### Complete Flow (with issues)

```bash
1. feature/x (#123) → develop
   ✅ Build + Test + PR + Issue commented

2. develop → release/vX.Y.Z → main
   ✅ Version + Tag + Changelog + Issue #123 closed

Automatic traceability!
```

---

## 📚 References

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Workflow Syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Closing Issues via Commit Messages](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue)
- [GitHub CLI](https://cli.github.com/manual/)

---

**Maintainers**: @ElioNeto

**Last updated**: March 6, 2026
