---
description: Marca issue como done no backlog local e atualiza backlog.json.
---

Argumento: $ARGUMENTS (número da issue, ex: `42`)

**Passo 1 — verificar argumento:**
!`[ -n "$ARGUMENTS" ] && echo "issue=$ARGUMENTS" || echo "ERRO: informe o número da issue"`

**Passo 2 — marcar como done:**
!`jq --argjson n $ARGUMENTS '(.issues[] | select(.number == $n) | .status) = "done"' .opencode/state/backlog.json > .opencode/state/backlog.tmp && mv .opencode/state/backlog.tmp .opencode/state/backlog.json`

**Passo 3 — confirmar:**
!`jq '.issues[] | select(.status == "done") | {number, title, status}' .opencode/state/backlog.json`

Responda com:

```
DONE: #<N> marcada como done
REMAINING: <total open>
NEXT: run orchestrator or /next-issue
```
