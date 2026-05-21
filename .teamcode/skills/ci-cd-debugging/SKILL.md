---
name: ci-cd-debugging
description: Guidelines for diagnosing and fixing CI/CD pipeline failures in the ApexStore project.
---

# CI/CD Debugging Skill

## Pipeline Architecture

- `.github/workflows/pr-validation.yml` — PR validation (fmt, clippy, audit, test, build_and_docs, report-status)
- `.github/workflows/ci.yml` — Workflow validation (actionlint, report-status)
- `.github/actions/ci-issue-manager/` — Auto-creates issues on CI failure
- `scripts/workflow-agent.ts` — Runs CI jobs locally for pre-PR validation
- `scripts/check-todos.ts` — Verifies task-state TODOs

## CI Failure Diagnosis Flow

1. Check which job failed (fmt, clippy, audit, test, build_and_docs)
2. Read the error block from the CI issue or workflow logs
3. Common failures:
   - **Type inference**: `as_ref()` ambiguity on `Vec<u8>` → use `as_slice()`
   - **Benchmark compilation**: `Arc::new(GlobalBlockCache::new())` → double Arc
   - **API mismatch**: `scan_range` parameter changes
   - **Formatting**: `cargo fmt` not run before commit
4. Fix → `cargo test --all-features` → `cargo clippy --all-targets --all-features`
5. Run `npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml` to validate

## Local Pre-PR Validation

```bash
npx tsx scripts/check-todos.ts .task-state.json   # verify TODOs
npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml  # run CI locally
```
