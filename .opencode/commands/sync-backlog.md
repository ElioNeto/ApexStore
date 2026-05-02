---
description: Sincroniza issues abertas do GitHub e atualiza .opencode/state/backlog.json com metadados compactos.
---

Sincronize o backlog com as issues abertas do repositório atual.

**Passo 1 — garantir diretório de estado:**
!`mkdir -p .opencode/state`

**Passo 2 — buscar issues abertas via MCP GitHub:**

Use o MCP do GitHub para listar issues abertas com labels e corpo. Para cada issue, extraia:
- `number`
- `priority` (da label: `critical|high|medium|low`; se nenhuma, `low`)
- `title`
- `status`: `open`
- `depends_on`: array de números extraídos de "Depends on" ou "depends on #N" no corpo
- `blocks`: array de números extraídos de "Blocks" ou "blocks #N" no corpo
- `acceptance_summary`: bullets da seção "Acceptance criteria" (máx 5 itens, texto curto)
- `fetched_body`: false

**Passo 3 — escrever estado:**
!`cat > .opencode/state/backlog.json << 'BACKLOG_EOF'
{"last_sync":"$(date -u +%Y-%m-%dT%H:%M:%SZ)","issues":[]}
BACKLOG_EOF`

Substitua o JSON acima com o conteúdo real gerado no Passo 2.

**Passo 4 — confirmar:**
!`jq '{total: (.issues | length), by_priority: (.issues | group_by(.priority) | map({(.[0].priority): length}) | add)}' .opencode/state/backlog.json`

Responda com:

```
SYNC: OK|FAILED
ISSUES: <total>
BY_PRIORITY:
- critical: N
- high: N
- medium: N
- low: N
NEXT: run orchestrator to start delivery loop
```
