## Regras de CI/CD

### workflow-agent
O script `scripts/workflow-agent.ts` executa localmente os jobs do `ci.yml` que não dependem de secrets externos.

Saída JSON linha a linha:
- `job_started` — início de um job
- `step_started` / `step_finished` — início/fim de cada step com `exitCode`
- `job_finished` — status do job (`success` | `failed` | `skipped`)
- `workflow_finished` — resultado final

Jobs pulados automaticamente quando requerem secrets externos:
- `secrets-scan`, `semgrep`, `sonarcloud` e similares

### check-todos
O script `scripts/check-todos.ts` verifica se os arquivos listados nos TODOs do `.task-state.json` existem.

Saída JSON: `{ ok: boolean, totals: {...}, results: [...] }`

### Pré-requisitos locais
- Docker disponível no PATH
- Node.js ≥ 20
- `cd scripts && npm install`
