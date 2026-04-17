#!/usr/bin/env bash
# Stop: valida Definition of Done antes de concluir.
# Roda cargo fmt --check, cargo clippy e verifica marcadores temporarios.
# Retorna exit 2 para bloquear encerramento se houver falha critica.

set -euo pipefail

FAILURES=()
WARNINGS=()

if ! command -v cargo &>/dev/null; then
  echo "[DoD] cargo nao encontrado - gates de qualidade ignorados." >&2
  exit 0
fi

echo "[DoD] Verificando formatacao (cargo fmt --check)..." >&2
if ! cargo fmt --all -- --check &>/dev/null; then
  FAILURES+=("FALHA Formatacao: rode 'cargo fmt --all' antes de concluir.")
fi

echo "[DoD] Verificando lint (cargo clippy)..." >&2
CLIPPY_OUT=$(cargo clippy --all-targets --all-features --message-format=short 2>&1 | grep '^error' | head -5 || true)
if [ -n "$CLIPPY_OUT" ]; then
  FAILURES+=("FALHA Clippy errors:\n$CLIPPY_OUT")
fi

echo "[DoD] Verificando TODO/FIXME/dbg! em src/ e tests/..." >&2
TODO_HITS=$(grep -rn --include='*.rs' -E '(TODO|FIXME|dbg!|eprintln!.*debug|unimplemented!|todo!)' src/ tests/ 2>/dev/null | head -10 || true)
if [ -n "$TODO_HITS" ]; then
  WARNINGS+=("AVISO Marcadores temporarios encontrados:\n$TODO_HITS")
fi

if [ ${#FAILURES[@]} -gt 0 ]; then
  echo "" >&2
  echo "===================================================" >&2
  echo "  [DoD] TASK NAO CONCLUIDA - corrija antes          " >&2
  echo "===================================================" >&2
  for f in "${FAILURES[@]}"; do
    echo -e "  $f" >&2
  done
  for w in "${WARNINGS[@]}"; do
    echo -e "  $w" >&2
  done
  exit 2
fi

if [ ${#WARNINGS[@]} -gt 0 ]; then
  echo "" >&2
  echo "[DoD] OK Gates passaram - mas veja os avisos:" >&2
  for w in "${WARNINGS[@]}"; do
    echo -e "  $w" >&2
  done
fi

echo "[DoD] OK Definition of Done satisfeito." >&2
exit 0
