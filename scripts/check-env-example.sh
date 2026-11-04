#!/usr/bin/env bash
#
# Fail when `.env.example` drifts from the environment variables the code
# actually reads. Two failure modes are caught:
#
#   1. A variable is read by `env::var(...)` but not documented -> operators
#      cannot discover it.
#   2. A variable is documented but read nowhere -> operators set it and
#      silently get no effect. This is how `DIR_PATH=/data` in
#      docker-compose.yml ended up being ignored while the server read
#      `DATA_DIR`, sending every write to a non-persistent path.
#
# Usage: scripts/check-env-example.sh   (run from the repository root)

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ ! -f .env.example ]]; then
    echo "error: .env.example not found" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Variables the Rust code reads.
grep -rhoE 'env::var\("[A-Z0-9_]+"\)' src --include='*.rs' \
    | sed -E 's/env::var\("(.*)"\)/\1/' \
    | sort -u > "$tmp/code.txt"

# Variables documented as assignments in .env.example. Lines inside the
# "Not yet wired" section are prefixed with `#`, so they are excluded here on
# purpose: they document known gaps rather than live settings.
grep -oE '^[A-Z0-9_]+=' .env.example | tr -d '=' | sort -u > "$tmp/doc.txt"

undocumented=$(comm -23 "$tmp/code.txt" "$tmp/doc.txt")
unread=$(comm -13 "$tmp/code.txt" "$tmp/doc.txt")

status=0

if [[ -n "$undocumented" ]]; then
    echo "error: read by the code but missing from .env.example:" >&2
    echo "$undocumented" | sed 's/^/  - /' >&2
    status=1
fi

if [[ -n "$unread" ]]; then
    echo "error: present in .env.example but read nowhere in src/:" >&2
    echo "$unread" | sed 's/^/  - /' >&2
    echo "  (move them to the 'Not yet wired' comment block, or wire them up)" >&2
    status=1
fi

if [[ $status -eq 0 ]]; then
    echo "ok: .env.example matches the $(wc -l < "$tmp/code.txt" | tr -d ' ') variables read by src/"
fi

exit $status
