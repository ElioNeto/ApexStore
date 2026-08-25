# ApexStore CI/CD Workflows

## Overview

Trunk-based development: features land in short-lived branches and merge directly
into `main`. Four workflows cover validation, benchmarking, release and docs.

## Workflow architecture

```text
feature/xyz
     |
     |  open PR to main
     v
  CI (ci.yml) ........... every job below must pass
     |
     |  merge to main
     v
  CI (ci.yml) ........... runs again on the merge commit
     |
     |  workflow_run: CI concluded successfully
     v
  Auto Release (release.yml)
     - bump patch version in Cargo.toml
     - verify the release builds with the locked dependencies
     - tag, GitHub release, `cargo publish --locked`

  Deploy Documentation (deploy-docs.yml) .... on push to main
  Benchmarks (benchmarks.yml) ............... nightly at 02:00 UTC
```

The release gate is the important edge. `release.yml` used to trigger directly on
`push: main` with no dependency on any test job, so a commit that broke the build
was still version-bumped, tagged, released and published to crates.io. A
crates.io publish cannot be replaced, only yanked, so it now waits for a
successful `CI` run via `workflow_run`.

## Active workflows

### `ci.yml` — CI

**Trigger:** `pull_request` to `main`, `push` to `main`, `workflow_dispatch`.

| Job | What it checks |
|-----|----------------|
| `Workflow lint` | `actionlint` over `.github/workflows/` |
| `Rustfmt` | `cargo fmt --all -- --check` |
| `Clippy` | `cargo clippy --locked --all-targets --all-features -- -D warnings` |
| `Test (dev)` | `cargo test --locked --all-features --workspace` |
| `Test (release)` | the same suite under `--release` |
| `Build and docs` | release build, examples, `cargo doc` |
| `MSRV` | `cargo check --all-features` at the `rust-version` in `Cargo.toml` |
| `Security audit` | `cargo audit --deny warnings` and `cargo deny check` |
| `.env.example drift` | `scripts/check-env-example.sh` |
| `Frontend` | `npm ci`, build, test, advisory `npm audit` |
| `Report status` | opens/updates a tracking issue on failure (push only) |

Notes on specific jobs:

- **`Test (release)`** exists because the `release` profile changes `panic`
  behaviour and the optimisation level. A dev-only suite exercises code paths
  that cannot occur in production.
- **`MSRV`** must use `--all-features`: the default feature set has a lower
  floor, so a default-only check would not detect the real MSRV. It clears
  `RUSTFLAGS` so a new lint in the pinned compiler cannot fail it — warnings are
  the `Clippy` job's business.
- **`Security audit`** pins `cargo-audit@0.22.2`. Versions before 0.22 abort with
  `unsupported CVSS version: 4.0` on the current advisory database, which is how
  two live advisories sat unreported in `Cargo.lock`.

### `release.yml` — Auto Release

**Trigger:** `workflow_run` on a successful `CI` run for `main`, or manual
dispatch.

1. Read the current version from `Cargo.toml`.
2. Increment the **patch** component.
3. `cargo update --locked --package apex-store-rs` — only the package version
   moves. The previous `cargo update --workspace` also bumped dependencies,
   releasing a tree CI never compiled.
4. `cargo build --locked --release --all-features` as a final gate.
5. Commit `chore: bump version to X.Y.Z [skip ci]`, tag `vX.Y.Z`.
6. GitHub release with generated notes, then `cargo publish --locked`.

Concurrency group `release` with `cancel-in-progress: false`: two concurrent runs
would bump from the same base version and race on `git push origin main`.

### `deploy-docs.yml` — Deploy Documentation

**Trigger:** push to `main`, or manual dispatch.

Builds the mdBook from `docs/` and publishes to GitHub Pages. mdBook is installed
through `taiki-e/install-action` rather than a raw `curl | tar`, because mdBook
publishes no checksum file and an unverified download cannot be validated.

A warning step lists any file under `docs/` that `docs/SUMMARY.md` does not
reference — mdBook renders only what `SUMMARY.md` lists, so an unlisted document
is silently unpublished.

### `benchmarks.yml` — Benchmarks

**Trigger:** nightly at 02:00 UTC, or manual dispatch. Deliberately **not** on
push or pull request: seven matrix jobs on every commit is slow, and the previous
configuration produced a stream of no-op `docs: update benchmark results` commits
on `main`.

The matrix covers all seven declared benchmarks. `latency_bench` and
`write_amplification` were declared in `Cargo.toml` but missing from the matrix,
so they had never run.

Results are published to `docs/PERFORMANCE.md`, not `README.md`: generated
content in a hand-maintained file is what created the commit churn. The publish
job commits only when the rendered table actually changed.

Failure handling is strict. The run step has no `continue-on-error`, a benchmark
that emits no `time:` lines fails the job, and an empty rendered table fails the
publish job with the raw input dumped into the log. The previous version masked
failures and wrote a `*No results parsed*` placeholder instead.

## Pinning

Every third-party action is pinned to a full commit SHA, with the version in a
trailing comment:

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1  # v7.0.1
```

Tags are mutable, so a retagged or compromised release would otherwise execute
with whatever permissions the job holds. `.github/dependabot.yml` keeps the pins
current through reviewable PRs — without it, pinning freezes them forever.

The Rust toolchain is pinned in `rust-toolchain.toml` (1.92, the floor required
by `wasmtime 44` and the cranelift crates under `--all-features`). Previously
every job used `container: rust:latest` *and* `dtolnay/rust-toolchain@stable`,
which was both redundant and unpinned: a new stable release could turn CI red
with no change to the repository.

`permissions` defaults to `contents: read` at workflow level, and jobs that need
more request it explicitly.

## Running the pipeline locally

`make ci` runs the same checks inside the toolchain image, so a red CI is
reproducible without pushing. See [`docs/DEVELOPMENT.md`](../../docs/DEVELOPMENT.md).

```bash
make ci          # fmt-check, clippy, test, audit, env-check, doc
make bench       # all seven benchmarks
make ci-local    # the CI workflow itself, via act
```

## Development flow

1. Branch from `main`.
2. Open a PR — `CI` runs every job above.
3. Merge — `CI` runs again on the merge commit, and a successful run triggers
   `Auto Release`.

See [`MIGRATION_GUIDE.md`](../../MIGRATION_GUIDE.md) for the team workflow.
