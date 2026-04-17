#!/usr/bin/env bash
# PreToolUse/File: bloqueia edição de arquivos sensíveis e protegidos.
# Lê o JSON do evento via stdin; extrai o campo file_path.

set -euo pipefail

# Lê o evento completo do stdin
INPUT=$(cat)

# Extrai o caminho do arquivo (compatível com jq e com python3 como fallback)
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

# --- Padrões sensíveis ---
SENSITIVE_PATTERNS=(
  '\.env$'
  '\.env\.'
  'secrets'
  '\.secret'
  'token'
  'private_key'
  '\.pem$'
  '\.key$'
  '\.pfx$'
  '\.p12$'
  '^prod'
  '/prod/'
  '\.git/'
)

# --- Arquivos protegidos (requerem justificativa explícita) ---
PROTECTED_PATTERNS=(
  'Cargo\.lock$'
  '\.github/workflows/'
  'migrations/'
  'docker-compose\.prod'
)

for pattern in "${SENSITIVE_PATTERNS[@]}"; do
  if echo "$FILE_PATH" | grep -qE "$pattern"; then
    echo "[BLOQUEADO] Arquivo sensível: $FILE_PATH" >&2
    echo "Motivo: corresponde ao padrão '$pattern'. Peça permissão explícita para editar arquivos sensíveis." >&2
    exit 2
  fi
done

for pattern in "${PROTECTED_PATTERNS[@]}"; do
  if echo "$FILE_PATH" | grep -qE "$pattern"; then
    echo "[AVISO] Arquivo protegido: $FILE_PATH" >&2
    echo "Motivo: '$pattern' requer justificativa clara no contexto antes de alterar." >&2
    # warning apenas — não bloqueia, mas registra
    exit 0
  fi
done

exit 0
