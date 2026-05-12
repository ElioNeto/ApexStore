.PHONY: ci-local bench-local ci-dry

# Run full CI pipeline locally via act (Docker)
ci-local:
	act -W .github/workflows/ci.yml

# Run benchmark workflow locally via act
bench-local:
	act -W .github/workflows/benchmarks.yml

# Dry-run: list workflow jobs without executing
ci-dry:
	act -W .github/workflows/ci.yml --dryrun
