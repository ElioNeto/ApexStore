---
description: Auditoria rápida do que está consumindo contexto na sessão atual.
---

Auditoria de uso de contexto.

**Passo 1 — listar arquivos abertos/lidos na sessão:**
!`git diff --name-only HEAD 2>/dev/null || echo 'sem diff'`

**Passo 2 — tamanho dos arquivos de estado:**
!`wc -l .opencode/state/backlog.json .task-state.json 2>/dev/null || echo 'arquivos ausentes'`

**Passo 3 — tamanho dos prompts dos agents:**
!`wc -l base/.opencode/agents/*.md 2>/dev/null || wc -l .opencode/agents/*.md 2>/dev/null || echo 'agents não encontrados'`

Com base na saída, responda:

```
CONTEXT AUDIT:
- backlog.json: <N linhas>
- task-state.json: <N linhas>
- agents: <maior agent e tamanho>
- diff aberto: <arquivos>

RISK:
- <item de maior consumo>

SUGGEST:
- <ação para reduzir se necessário>
```
