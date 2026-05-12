# Completed Issues

## #119 — [BUG] Pipeline CI não validado
**Status:** ✅ Complete — commit: `5eccddd` + `c54fa45`
**Changes:**
- Fixed `unwrap()` in `lock_core()` → returns `Result<LockPoisoned>`
- Fixed `expect()` in `Table::clone()` → graceful bloom filter error handling
- All CI pipeline tools validated: `cargo test --all-features` (114 tests pass), `cargo clippy`, `cargo audit`, `workflow-agent`, `check-todos`

## #120 — [BUG] Testes de falha ausentes
**Status:** ✅ Complete — commit: `5eccddd`
**Changes:**
- WAL truncation recovery tests: graceful truncation, mid-write, partial replay
- SSTable corruption tests → `LsmError::CorruptedData`
- Compaction crash consistency tests
- Latency benchmarks: P50/P95/P99 for 1k and 100k keys
- Write amplification benchmarks: <10x Leveled, <3x Size-Tiered

## #121 — [REFACTOR] Mover histórico LsmError para CHANGELOG.md
**Status:** ✅ Complete — commit: `1152373` + `bb073e4`
**Changes:**
- Added [Unreleased] entries in CHANGELOG.md
- Removed variant history table from `error.rs`, pointing to CHANGELOG
- Added docstring examples for `Engine`, `LsmConfig`, `CompactionStrategy`
- Created `examples/basic.rs` with open → put → get → delete

## #124 — [BUG] search() stubs retornando Vec::new()
**Status:** ✅ Complete — commit: `ecbd7e3`
**Changes:**
- Removed `search()` and `search_prefix_legacy()` from public API
- Added CHANGELOG entry documenting breaking change
- Audited all public functions for silent `Vec::new()` returns

## #125 — [PERF] Latência de leitura acima de 1 ms
**Status:** ✅ Complete — commit: `5eccddd`
**Changes:**
- Bloom filter checked before SSTable I/O in `get_cf`
- Block cache (`GlobalBlockCache`) configurable via `LsmConfig.block_cache_size_mb`
- `scan_cf` skips SSTables whose `[min_key, max_key]` do not intersect query range
- Merge iterators use `BinaryHeap` (min-heap) for O(log N) merge

## #128 — [CI] Simular GitHub Actions localmente com Docker
**Status:** ✅ Complete — uncommitted files
**Changes:**
- `.actrc` configured with `catthehacker/ubuntu:act-latest`
- `.secrets.example` created, `.secrets` added to `.gitignore`
- `Makefile` with `ci-local`, `bench-local`, `ci-dry` targets
- `ci.yml` rewritten with `validate-workflows` job using `rhysd/actionlint-action@v1`

## #127 — [CI] Abrir/fechar issue automaticamente ao falhar/sucesso
**Status:** ✅ Complete — commit: `2a9f74a`
**Changes:**
- Created `.github/actions/ci-issue-manager/action.yml` composite action
- Creates issue with `ci-failure` label on job failure
- Closes issue with success comment when job passes
- Filters issues by workflow name prefix
- Added `report-status` job to all 5 workflow files

## #126 — [Benchmark] Formatar resultados com OpenCode no README
**Status:** ✅ Complete — commit: `c1586c2` + `4a60b58`
**Changes:**
- Created `scripts/format-benchmarks.sh` with 3-tier fallback
- Modified `benchmarks.yml` to install OpenCode and format results
- Updated `.gitignore` to track `format-benchmarks.sh` despite `*.sh` rule
- `README.md` updated with benchmark results between markers
