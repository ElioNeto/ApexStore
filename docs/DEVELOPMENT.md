# Development Environment

ApexStore builds and tests inside a container, so contributors do not need a local
Rust toolchain. The image is defined by [`Dockerfile.dev`](../Dockerfile.dev) and
every command below is also available as a `make` target.

## Why a container

The crate's `--all-features` build pulls in `wasmtime 44` and the cranelift crates,
which declare `rust-version = 1.92.0`. Building with an older toolchain fails with
roughly forty `requires rustc 1.92.0` lines and no obvious cause. Pinning the
toolchain in an image removes that class of "works on my machine" report, and gives
CI and contributors the same compiler, the same `cargo-audit`, and the same
`cargo-deny`.

This is a *toolchain* image. The production server image is the separate
multi-stage [`Dockerfile`](../Dockerfile).

## One-time setup

```bash
make dev-image
```

That builds `apexstore-dev` (~3.5 GB, mostly the Rust toolchain plus the LLVM
headers `wasmtime` needs). Rebuild it only when `Dockerfile.dev` changes.

## Everyday commands

| Command | What it runs |
|---------|--------------|
| `make fmt` | `cargo fmt --all` |
| `make fmt-check` | `cargo fmt --all -- --check` |
| `make clippy` | `cargo clippy --locked --all-targets --all-features -- -D warnings` |
| `make test` | `cargo test --locked --all-features --workspace` |
| `make test-fast` | library unit tests only |
| `make bench` | all Criterion benchmarks |
| `make audit` | `cargo audit` against the RustSec advisory database |
| `make deny` | `cargo deny check` (licences, bans, sources) |
| `make doc` | `cargo doc --locked --no-deps --all-features` |
| `make env-check` | verifies `.env.example` matches the variables `src/` reads |
| `make actionlint` | lints `.github/workflows/` |
| `make ci` | all of the above, in the order CI runs them |
| `make shell` | interactive shell in the toolchain image |

Run `make help` for the full list.

## How caching works

Two named Docker volumes are mounted on every run:

- `apexstore-cargo-registry` → `/usr/local/cargo/registry` (downloaded crates)
- `apexstore-target` → `/app/target` (compiled artifacts)

The repository itself is bind-mounted at `/app`, so edits on the host are visible
immediately and build output never pollutes the working tree. A cold
`make clippy` compiles ~573 crates and takes several minutes; subsequent runs are
incremental.

`make clean-volumes` deletes both volumes when you need a genuinely cold build.

## Running commands directly

`make` is a thin wrapper. The equivalent raw invocation:

```bash
docker run --rm \
  -v "$PWD:/app" \
  -v apexstore-cargo-registry:/usr/local/cargo/registry \
  -v apexstore-target:/app/target \
  apexstore-dev cargo test --all-features
```

On Windows with Git Bash, prefix with `MSYS_NO_PATHCONV=1` and use a Windows-style
path (`-v "C:/path/to/apexstore:/app"`) so the path is not rewritten.

Use `bash -c "..."` rather than `bash -lc "..."`: a login shell resets `PATH` and
`cargo` disappears.

## Working without the container

A host toolchain works too. `rust-toolchain.toml` pins **1.92**, so `rustup` will
install and select it automatically on first `cargo` invocation in this
repository — no manual `rustup override` needed. The same version is declared as
`rust-version` in `Cargo.toml` and as the base image of `Dockerfile.dev`; keep the
three in sync.

Install `cargo-audit 0.22` or later — version 0.21 cannot parse CVSS 4.0 entries
in the current advisory database and aborts with
`unsupported CVSS version: 4.0`.

## The four binaries

```
apexstore-server   src/bin/server.rs   REST + GraphQL API server (the default-run target)
apexstore-cli      src/bin/cli.rs      CLI / REPL
apexstore-tui      src/bin/tui.rs      terminal dashboard
apex-store-rs      src/main.rs         13-line stub; see the issue tracker
```

Always pass `--bin <name>`. `cargo run` on its own resolves to
`apexstore-server` via the `default-run` key in `Cargo.toml`.

## Frontend

The Angular dashboard in [`frontend/`](../frontend) has its own toolchain and is
not covered by this image or by CI:

```bash
cd frontend
npm ci
npm run build
npm test
```

## Benchmarks

```bash
make bench
```

All seven benchmarks honour a `CI` environment variable: when it is set they
reduce the sample count and shrink the datasets, so a full pass takes minutes
rather than hours.

```bash
CI=true cargo bench --bench read_bench -- --noplot
```

Note that `Criterion::sample_size` asserts `n >= 10` and panics below that, so 10
is the floor for the `CI` branch in each `configure_criterion`.

## Fuzzing

`cargo-fuzz` needs a nightly toolchain and is therefore not in the image:

```bash
cargo +nightly fuzz run wal -- -runs=10000
cargo +nightly fuzz run sstable -- -runs=10000
```
