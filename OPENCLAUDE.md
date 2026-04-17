# OPENCLAUDE.md - Diferencas OpenClaude vs Claude Code

Este arquivo documenta as diferencas de configuracao entre `.claude/` (Claude Code)
e `.openclaude/` (OpenClaude), e como a governanca se comporta em cada runtime.

---

## Arquitetura da governanca

```
.claude/
  settings.json             <- lido pelo Claude Code
  hooks/
    pre-tool-file.sh        <- guarda de arquivos (Claude Code)
    pre-tool-bash.sh        <- guarda de comandos (Claude Code)
    post-tool-lint.sh       <- lint pos-edicao (Claude Code)
    stop-dod.sh             <- Definition of Done (Claude Code)

.openclaude/
  settings.json             <- lido pelo OpenClaude
  hooks/
    pre-tool-file.sh        <- guarda de arquivos (OpenClaude)
    pre-tool-bash.sh        <- guarda de comandos (OpenClaude)
    post-tool-lint.sh       <- lint pos-edicao (OpenClaude)
    stop-dod.sh             <- Definition of Done (OpenClaude)
  CLAUDE.md
  commands/
  skills/
  memory.md
  decisions.md
  error-catalog.md
  pr-checklist.md
```

**Decisao de design:** cada runtime tem seus proprios scripts em seu proprio diretorio.
Isso evita acoplamento entre as duas ferramentas e garante que cada uma possa
evoluir de forma independente se os formatos ou comportamentos divergirem.

---

## Diferencas de comportamento

| Aspecto | Claude Code | OpenClaude |
|---|---|---|
| Config lida | `.claude/settings.json` | `.openclaude/settings.json` |
| Scripts de hook | `.claude/hooks/*.sh` | `.openclaude/hooks/*.sh` |
| Matcher de ferramenta | `Edit`, `Write`, `MultiEdit`, `Bash` | `file_edit`, `file_write`, `bash` |
| Contexto de projeto | `CLAUDE.md` na raiz | `.openclaude/CLAUDE.md` |
| Memoria de sessao | Nativa do Claude Code | `.openclaude/memory.md` |
| Comandos slash | `.claude/commands/` | `.openclaude/commands/` |

### Por que matchers diferentes?

O Claude Code usa nomes PascalCase para ferramentas (`Edit`, `Write`, `Bash`).
O OpenClaude usa snake_case (`file_edit`, `file_write`, `bash`).
Cada `settings.json` ja esta configurado com os nomes corretos para seu runtime.

---

## Sincronizando mudancas entre os dois diretorios

Como os scripts sao copias independentes, uma mudanca de logica deve ser aplicada
em ambos. Para simplificar:

```bash
cp .claude/hooks/pre-tool-file.sh   .openclaude/hooks/pre-tool-file.sh
cp .claude/hooks/pre-tool-bash.sh   .openclaude/hooks/pre-tool-bash.sh
cp .claude/hooks/post-tool-lint.sh  .openclaude/hooks/post-tool-lint.sh
cp .claude/hooks/stop-dod.sh        .openclaude/hooks/stop-dod.sh
```

---

## Adicionando um novo hook

1. Crie o script em `.claude/hooks/meu-hook.sh`
2. Copie para `.openclaude/hooks/meu-hook.sh`
3. Adicione a entrada em `.claude/settings.json` (matcher PascalCase)
4. Adicione a entrada em `.openclaude/settings.json` (matcher snake_case)
5. Documente em `CLAUDE.md` e aqui

---

## Referencia rapida de eventos

| Evento | Quando dispara | Pode bloquear? |
|---|---|---|
| `PreToolUse` | Antes de executar qualquer ferramenta | Sim (exit 2) |
| `PostToolUse` | Apos ferramenta executar com sucesso | Nao bloqueia a ferramenta ja executada |
| `Stop` | Antes do agente encerrar a resposta | Sim (exit 2) |
