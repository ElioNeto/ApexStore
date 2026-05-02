---
description: Propaga atualizações deste boilerplate para um ou mais repositórios alvo abrindo PRs.
---

Atualize o boilerplate OpenCode nos repositórios alvo.

Argumentos: $ARGUMENTS
(formato: `owner/repo` separados por espaço, ou `all` para usar a lista em `.opencode/state/target-repos.json`)

**Passo 1 — resolver lista de targets:**

Se `$ARGUMENTS` for `all`:
!`cat .opencode/state/target-repos.json 2>/dev/null || echo '{"repos":[]}'`

Se `$ARGUMENTS` contiver repos explícitos, use-os diretamente.

**Passo 2 — para cada repositório alvo:**

Use o MCP do GitHub para:
1. Verificar se o repositório existe e está acessível.
2. Criar branch `boilerplate/update-YYYYMMDD` a partir do branch padrão.
3. Copiar os seguintes arquivos deste boilerplate para o repo alvo:
   - `base/.opencode/agents/*.md` → `.opencode/agents/`
   - `base/.opencode/commands/*.md` → `.opencode/commands/`
   - `base/.opencode/ignore` → `.opencode/ignore`
   - `base/opencode.json` → `opencode.json` (somente se não existir — não sobrescrever)
4. Abrir PR com:
   - Título: `chore: update opencode boilerplate (YYYY-MM-DD)`
   - Body: lista de arquivos atualizados + link para este repositório.

**Passo 3 — confirmar:**

Responda com:

```
UPDATE SUMMARY:
- <owner/repo>: PR #N criada|SKIPPED (<motivo>)|FAILED (<erro>)

NEXT: revisar e mergear as PRs abertas
```
