---
description: Verifica TODOs e executa a pipeline local para fechamento da task.
---

Execute a validação de entrega desta tarefa.

**Passo 1 — verificar TODOs:**
!`npx tsx scripts/check-todos.ts .task-state.json 2>&1 || echo '{"ok":false,"error":"task-state.json nao encontrado"}'`

**Passo 2 — executar a pipeline local:**
!`npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml 2>&1`

Com base na saída acima, responda obrigatoriamente com:

```
STATUS: SUCCESS|FAILED

TODOs:
- [x] id: título
- [ ] id: título (pendente: motivo)

JOBS:
- <job>: SUCCESS|FAILED|SKIPPED — motivo se falhou

NEXT: FINALIZAR|<patch necessário>
```

Argumentos extras: $ARGUMENTS
