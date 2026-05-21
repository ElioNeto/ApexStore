---
description: "Run a Rust dependency audit — checks for outdated, vulnerable, or conflicting dependencies in Cargo.toml and Cargo.lock"
agent: deps
subtask: true
---

Run a dependency audit across the ApexStore Rust project.

Use the @deps agent to:

1. Read `Cargo.toml` and `Cargo.lock`
2. Check for:
   - Security vulnerabilities (via `cargo audit`)
   - Outdated dependencies (via `cargo outdated`)
   - Duplicate dependencies (via `cargo tree -d`)
   - Unused dependencies (via `cargo udeps` — nightly only)
3. Generate a comprehensive `dependency-audit-report.md`

$ARGUMENTS

Focus areas if specified in arguments:
- Specific crates to audit
- Severity levels to prioritize
- Output format preferences
