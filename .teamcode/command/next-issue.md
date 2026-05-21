---
description: Mostra a próxima issue desbloqueada por prioridade sem iniciar implementação.
---

**Passo 1 — carregar estado:**
!`cat .opencode/state/backlog.json 2>/dev/null || echo '{"issues":[]}'`

Com base no JSON acima:

1. Filtrar issues com `status: open`.
2. Identificar issues cujas dependências (`depends_on`) estão todas com `status: done`.
3. Ordenar: `critical > high > medium > low`, empate pelo menor número.
4. Selecionar a primeira.

Responda com:

```
NEXT: <numero|none>
TITLE: <título>
PRIORITY: <prioridade>
DEPENDS_ON: <lista|none>
BLOCKS: <lista|none>
ACCEPTANCE:
- <critério>
READY: yes|no — <motivo se no>
```
