# ApexStore -- developer entry points.
#
# Everything Rust runs inside the container image built from `Dockerfile.dev`,
# so no local toolchain is required. See docs/DEVELOPMENT.md.
#
# Cargo's registry and target directory live in named Docker volumes, so
# repeated invocations reuse the previous build.

DEV_IMAGE  ?= apexstore-dev
CARGO_VOL  ?= apexstore-cargo-registry
TARGET_VOL ?= apexstore-target
REPO_DIR   ?= $(CURDIR)

# `docker run` incantation shared by every Rust target.
DOCKER_RUN = docker run --rm \
	-v "$(REPO_DIR):/app" \
	-v $(CARGO_VOL):/usr/local/cargo/registry \
	-v $(TARGET_VOL):/app/target \
	$(DEV_IMAGE)

.PHONY: help dev-image shell fmt fmt-check clippy test test-fast bench audit deny doc \
        env-check ci ci-local bench-local ci-dry docker-build docker-up docker-down clean-volumes

help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

# ── Toolchain image ──────────────────────────────────────────────────────────

dev-image: ## Build the Rust toolchain image used by every target below
	docker build -f Dockerfile.dev -t $(DEV_IMAGE) .

shell: ## Interactive shell inside the toolchain image
	docker run --rm -it \
		-v "$(REPO_DIR):/app" \
		-v $(CARGO_VOL):/usr/local/cargo/registry \
		-v $(TARGET_VOL):/app/target \
		$(DEV_IMAGE) bash

# ── Quality gates (same commands CI runs) ────────────────────────────────────

fmt: ## Format the workspace in place
	$(DOCKER_RUN) cargo fmt --all

fmt-check: ## Fail if the workspace is not formatted
	$(DOCKER_RUN) cargo fmt --all -- --check

clippy: ## Lint with warnings denied
	$(DOCKER_RUN) cargo clippy --locked --all-targets --all-features -- -D warnings

test: ## Full test suite
	$(DOCKER_RUN) cargo test --locked --all-features --workspace

test-fast: ## Library unit tests only
	$(DOCKER_RUN) cargo test --locked --lib --all-features

bench: ## Run all Criterion benchmarks
	$(DOCKER_RUN) cargo bench --locked --all-features

audit: ## Scan Cargo.lock against the RustSec advisory database
	$(DOCKER_RUN) cargo audit

deny: ## Check licences, bans and sources
	$(DOCKER_RUN) cargo deny check

doc: ## Build rustdoc without dependencies
	$(DOCKER_RUN) cargo doc --locked --no-deps --all-features

env-check: ## Fail if .env.example drifts from the variables the code reads
	$(DOCKER_RUN) bash scripts/check-env-example.sh

ci: fmt-check clippy test audit env-check doc ## Everything CI checks, locally

# ── Workflow debugging via `act` (runs on the host, needs act installed) ─────

ci-local: ## Run the CI workflow locally with act
	act -W .github/workflows/ci.yml

bench-local: ## Run the benchmarks workflow locally with act
	act -W .github/workflows/benchmarks.yml

ci-dry: ## List workflow jobs without executing them
	act -W .github/workflows/ci.yml --dryrun

# ── Runtime image ───────────────────────────────────────────────────────────

docker-build: ## Build the production server image
	docker build -t apexstore:latest .

docker-up: ## Start the server via docker compose
	docker compose up -d

docker-down: ## Stop the compose stack
	docker compose down

clean-volumes: ## Delete the cargo/target caches (forces a cold rebuild)
	docker volume rm -f $(CARGO_VOL) $(TARGET_VOL)
