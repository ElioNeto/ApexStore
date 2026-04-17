#!/usr/bin/env bash
# PreToolUse/File: bloqueia edicao de arquivos sensiveis e protegidos.
# Le o JSON do evento via stdin; extrai o campo file_path.

set -euo pipefail

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

PROTECTED_PATTERNS=(
  'Cargo\.lock$'
  '\.github/workflows/'
  'migrations/'
  'docker-compose\.prod'
)

for pattern in "${SENSITIVE_PATTERNS[@]}"; do
  if echo "$FILE_PATH" | grep -qE "$pattern"; then
    echo "[BLOQUEADO] Arquivo sensivel: $FILE_PATH" >&2
    echo "Motivo: corresponde ao padrao '$pattern'. Peca permissao explicita para editar arquivos sensiveis." >&2
    exit 2
  fi
done

for pattern in "${PROTECTED_PATTERNS[@]}"; do
  if echo "$FILE_PATH" | grep -qE "$pattern"; then
    echo "[AVISO] Arquivo protegido: $FILE_PATH" >&2
    echo "Motivo: '$pattern' requer justificativa clara no contexto antes de alterar." >&2
    exit 0
  fi
done

exit 0
