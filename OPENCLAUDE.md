# OPENCLAUDE.md — Diferenças OpenClaude vs Claude Code

Este arquivo documenta as diferenças de configuração entre `.claude/` (Claude Code)
e `.openclaude/` (OpenClaude), e como a governança se comporta em cada runtime.

---

## Arquitetura da governança

```
.claude/
  settings.json          ← lido pelo Claude Code
  hooks/                 ← scripts compartilhados por ambos os runtimes
    pre-tool-file.sh
    pre-tool-bash.sh
    post-tool-lint.sh
    stop-dod.sh

.openclaude/
  settings.json          ← lido pelo OpenClaude (aponta para os mesmos scripts)
  CLAUDE.md              ← contexto de projeto para o OpenClaude
  commands/              ← comandos slash customizados
  skills/                ← habilidades reutilizáveis
  memory.md              ← memória persistente de sessão
  decisions.md           ← registro de decisões arquiteturais
  error-catalog.md       ← catálogo de erros conhecidos
  pr-checklist.md        ← checklist de PR
```

**Decisão de design:** os scripts de hook ficam **apenas em `.claude/hooks/`**
e são referenciados pelo `.openclaude/settings.json` via caminho relativo.
Isso evita duplicação e garante que uma correção num script beneficia ambos os runtimes.

---

## Diferenças de comportamento

| Aspecto | Claude Code | OpenClaude |
|---|---|---|
| Config lida | `.claude/settings.json` | `.openclaude/settings.json` |
| Scripts de hook | `.claude/hooks/*.sh` | `.claude/hooks/*.sh` (mesmos) |
| Matcher de ferramenta | `Edit`, `Write`, `MultiEdit`, `Bash` | `file_edit`, `file_write`, `bash` |
| Contexto de projeto | `CLAUDE.md` na raiz | `.openclaude/CLAUDE.md` |
| Memória de sessão | Nativa do Claude Code | `.openclaude/memory.md` |
| Comandos slash | `.claude/commands/` | `.openclaude/commands/` |

### Matchers

O Claude Code usa nomes PascalCase para ferramentas (`Edit`, `Write`, `Bash`).
O OpenClaude usa snake_case (`file_edit`, `file_write`, `bash`).
Os dois `settings.json` já estão configurados com os nomes corretos para cada runtime.

---

## Compatibilidade dos hooks

Todos os scripts foram escritos em `bash` puro com:
- `jq` como parser JSON principal
- `python3` como fallback se `jq` não estiver disponível
- Degradação silenciosa se nenhum dos dois estiver disponível

Isso garante funcionamento em ambientes mínimos (containers, CI, máquinas novas).

---

## Quando só um runtime está em uso

Se você usa **apenas Claude Code**: o `.openclaude/` é ignorado — nenhum impacto.
Se você usa **apenas OpenClaude**: o `.claude/settings.json` é ignorado,
mas os scripts em `.claude/hooks/` ainda são usados via referência no `.openclaude/settings.json`.

Não há necessidade de duplicar scripts. A estrutura atual é coerente para ambos.

---

## Adicionando um novo hook

1. Crie o script em `.claude/hooks/meu-hook.sh`
2. Adicione a entrada em `.claude/settings.json` (Claude Code)
3. Adicione a entrada em `.openclaude/settings.json` (OpenClaude) com matcher no formato correto
4. Documente em `CLAUDE.md` e aqui

---

## Referência rápida de eventos

| Evento | Quando dispara | Pode bloquear? |
|---|---|---|
| `PreToolUse` | Antes de executar qualquer ferramenta | Sim (exit 2) |
| `PostToolUse` | Após ferramenta executar com sucesso | Não bloqueia a ferramenta já executada |
| `Stop` | Antes do agente encerrar a resposta | Sim (exit 2) |
