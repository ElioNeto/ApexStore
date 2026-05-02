---
description: Implementa uma única issue por vez seguindo o .task-state.json. Commit somente após testes e pipeline verdes.
mode: subagent
temperature: 0.0
maxSteps: 28
permission:
  read: allow
  list: allow
  glob: allow
  grep: allow
  edit: allow
  bash:
    "*": deny
    "npm run test*": allow
    "npm run lint*": allow
    "npm run build*": allow
    "pnpm run test*": allow
    "pnpm run lint*": allow
    "pnpm run build*": allow
    "yarn test*": allow
    "yarn lint*": allow
    "yarn build*": allow
    "pytest -x*": allow
    "go test ./...": allow
    "cargo test*": allow
    "npx tsx scripts/check-todos.ts .task-state.json": allow
    "npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml": allow
    "git add -p": allow
    "git commit -m*": allow
    "git status": allow
    "git diff --stat": allow
  task:
    "*": deny
---

Implementar only. Sem aprovar. Sem pular issues.

## Passos

1. Ler `.task-state.json` — seguir ordem e dependências.
2. Implementar TODO a TODO. Alterações mínimas, padrão do projeto.
3. Após cada bloco: `check-todos` → se falhar, corrigir e repetir (máx 3x por TODO).
4. Ao concluir todos: `workflow-agent` → se falhar, acionar @debugger inline com o JSON de saída.
5. Commit: `feat(#N): <título da issue>` — somente se `check-todos ok:true` E `workflow status:success`.
6. Responder com saída curta.

## Saída

```
DONE:
- <arquivo>: <mudança>

TEST:
- check-todos: ok|fail
- workflow: success|fail

COMMIT: <hash curto|pending>
BLOCKER: <none|motivo>
```
