#!/usr/bin/env bash
# PreToolUse/Bash: bloqueia comandos destrutivos ou arriscados.
# Lê o JSON do evento via stdin; extrai o campo command.

set -euo pipefail

INPUT=$(cat)

if command -v jq &>/dev/null; then
  CMD=$(echo "$INPUT" | jq -r '.tool_input.command // ""' 2>/dev/null || echo "")
else
  CMD=$(echo "$INPUT" | python3 -c "
import sys, json
d = json.load(sys.stdin)
print(d.get('tool_input', {}).get('command') or '')
" 2>/dev/null || echo "")
fi

[ -z "$CMD" ] && exit 0

# --- Padrões destrutivos (bloqueio hard) ---
declare -A BLOCK_RULES
BLOCK_RULES['rm -rf /']=      'rm -rf na raiz do sistema é proibido.'
BLOCK_RULES['rm -rf ~']=      'rm -rf no home é proibido.'
BLOCK_RULES['git push --force']='git push --force pode destruir histórico remoto. Use --force-with-lease.'
BLOCK_RULES['git push -f']=   'git push -f pode destruir histórico remoto. Use --force-with-lease.'
BLOCK_RULES['docker system prune']='docker system prune apaga volumes/imagens sem confirmação interativa.'
BLOCK_RULES['docker volume prune']='docker volume prune apaga dados persistentes.'
BLOCK_RULES['chmod -R 777']=  'chmod -R 777 é perigoso para segurança.'
BLOCK_RULES['chmod 777']=     'chmod 777 expõe o arquivo para todos os usuários.'

for pattern in "${!BLOCK_RULES[@]}"; do
  if echo "$CMD" | grep -qF "$pattern"; then
    echo "[BLOQUEADO] Comando perigoso detectado." >&2
    echo "Motivo: ${BLOCK_RULES[$pattern]}" >&2
    echo "Comando: $CMD" >&2
    exit 2
  fi
done

# --- Padrões de pipe suspeito (curl|sh, wget|sh) ---
if echo "$CMD" | grep -qE '(curl|wget).+\|.*(sh|bash|zsh)'; then
  echo "[BLOQUEADO] Pipe de download para shell detectado." >&2
  echo "Motivo: curl/wget|sh executa código remoto sem inspeção. Baixe o script primeiro e inspecione." >&2
  exit 2
fi

# --- rm -rf fora de /tmp ou target ---
if echo "$CMD" | grep -qE 'rm\s+-rf?\s+[^/]'; then
  if ! echo "$CMD" | grep -qE 'rm\s+-rf?\s+(\./)?((tmp|target|/tmp|/target))'; then
    echo "[AVISO] rm -rf em caminho não-temporário: $CMD" >&2
    echo "Confirme se o diretório é seguro para remoção." >&2
    # warning — não bloqueia
  fi
fi

exit 0
