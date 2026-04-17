#!/usr/bin/env bash
# PostToolUse/File: roda cargo fmt + cargo clippy no arquivo alterado.
# Faz fallback silencioso se cargo nao estiver disponivel.
# Usa flag de lock para evitar loop infinito de autoedicao.

set -euo pipefail

LOCK_FILE="/tmp/.apexstore_lint_running"

if [ -f "$LOCK_FILE" ]; then
  exit 0
fi

INPUT=$(cat)

if command -v jq &>/dev/null; then
  FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.path // ""' 2>/dev/null || echo "")
else
  FILE_PATH=$(echo "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
ti = d.get('tool_input', {})
print(ti.get('file_path') or ti.get('path') or '')
" 2>/dev/null || echo "")
fi

[ -z "$FILE_PATH" ] && exit 0

if ! echo "$FILE_PATH" | grep -qE '\.rs$'; then
  exit 0
fi

if ! command -v cargo &>/dev/null; then
  echo "[AVISO] cargo nao encontrado - lint ignorado. Instale rustup para habilitar gates locais." >&2
  exit 0
fi

touch "$LOCK_FILE"
trap 'rm -f $LOCK_FILE' EXIT

echo "[LINT] Rodando cargo fmt em $FILE_PATH..." >&2
if ! cargo fmt -- "$FILE_PATH" 2>&1 | tail -5 >&2; then
  echo "[AVISO] cargo fmt falhou em $FILE_PATH" >&2
fi

echo "[LINT] Rodando cargo clippy..." >&2
CLIPPY_OUT=$(cargo clippy --message-format=short 2>&1 | grep "$FILE_PATH" | head -10 || true)
if [ -n "$CLIPPY_OUT" ]; then
  echo "[CLIPPY] $CLIPPY_OUT" >&2
fi

exit 0
