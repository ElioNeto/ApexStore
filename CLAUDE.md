# CLAUDE.md — Governança Local do ApexStore

Este arquivo é lido automaticamente pelo Claude Code ao iniciar uma sessão neste repositório.
Documenta as regras de comportamento, hooks ativos e gates de qualidade.

---

## Stack detectada

- **Linguagem:** Rust (Cargo.toml + Cargo.lock)
- **Formatter:** `cargo fmt`
- **Linter:** `cargo clippy`
- **Testes:** `cargo test --all-features`
- **Build:** `cargo build --release`
- **Container:** Docker + docker-compose.yml
- **CI remoto:** GitHub Actions (`.github/workflows/pr-validation.yml`)

---

## Hooks ativos

Todos os scripts ficam em `.claude/hooks/` e são reutilizados pelo `.openclaude/settings.json`.

### `PreToolUse` — pre-tool-file.sh

Disparado antes de qualquer `Edit`, `Write` ou `MultiEdit`.

**Bloqueia (exit 2):**
- Arquivos sensíveis: `.env`, `.env.*`, `secrets`, `*.key`, `*.pem`, `*.p12`, caminhos com `prod/`, diretório `.git/`
- Padrão de token/secret no nome do arquivo

**Avisa (exit 0 + stderr):**
- `Cargo.lock` — lockfile não deve ser editado manualmente
- `.github/workflows/` — configs de CI requerem justificativa
- `migrations/` — migrations críticas
- `docker-compose.prod*`

### `PreToolUse` — pre-tool-bash.sh

Disparado antes de qualquer `Bash`.

**Bloqueia (exit 2):**
- `rm -rf /` ou `rm -rf ~`
- `git push --force` / `git push -f`
- `docker system prune` / `docker volume prune`
- `chmod -R 777` / `chmod 777`
- Qualquer `curl|sh` ou `wget|sh` (pipe de download para shell)

**Avisa:**
- `rm -rf` em caminhos fora de `tmp/` ou `target/`

### `PostToolUse` — post-tool-lint.sh

Disparado após `Edit`, `Write` ou `MultiEdit` em arquivos `.rs`.

- Roda `cargo fmt` no arquivo alterado
- Roda `cargo clippy` e filtra warnings do arquivo
- Usa lock em `/tmp/.apexstore_lint_running` para evitar loop infinito
- Se `cargo` não estiver disponível, avisa e segue sem bloquear

### `Stop` — stop-dod.sh

Disparado antes de o agente encerrar a resposta.

**Bloqueia (exit 2) se:**
- `cargo fmt --check` falha (código mal formatado)
- `cargo clippy` retorna erros (`error[...]`)

**Avisa se:**
- Há `TODO`, `FIXME`, `dbg!`, `unimplemented!` ou `todo!` em `src/` ou `tests/`

**Libera** se todos os gates passam.

---

## Definition of Done

Uma task é considerada completa apenas se:

- [ ] A solicitação principal foi atendida
- [ ] `cargo fmt --check` passa
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passa
- [ ] `cargo test --all-features` passa nas partes tocadas
- [ ] Nenhum `TODO/FIXME/dbg!/todo!/unimplemented!` foi deixado
- [ ] Documentação relevante foi atualizada se necessário

---

## Como desativar temporariamente

```bash
# Desativar um hook específico — comente o matcher no settings.json
# Ou use a variável de ambiente para pular o DoD:
export APEX_SKIP_DOD=1  # reconhecido pelo stop-dod.sh se quiser adicionar

# Desativar todos os hooks da sessão:
# Remova ou renomeie .claude/settings.json temporariamente
mv .claude/settings.json .claude/settings.json.bak
```

Para pular apenas o lint pós-edição:
```bash
touch /tmp/.apexstore_lint_running  # simula o lock
```

---

## Como evoluir para CI remoto

Os gates locais espelham o workflow `.github/workflows/pr-validation.yml`.
Para adicionar um novo gate:

1. Adicione o comando no hook shell correspondente em `.claude/hooks/`
2. Adicione o mesmo step em `.github/workflows/pr-validation.yml`
3. Documente aqui e em `OPENCLAUDE.md`

Exemplos de gates futuros:
```yaml
# pr-validation.yml
- name: Security audit
  run: cargo audit

- name: Coverage check
  run: cargo tarpaulin --fail-under 80
```

---

## Arquivos de governança

| Arquivo | Propósito |
|---|---|
| `.claude/settings.json` | Configuração de hooks para Claude Code |
| `.openclaude/settings.json` | Configuração de hooks para OpenClaude (reutiliza os mesmos scripts) |
| `.claude/hooks/pre-tool-file.sh` | Guarda de arquivos sensíveis/protegidos |
| `.claude/hooks/pre-tool-bash.sh` | Guarda de comandos destrutivos |
| `.claude/hooks/post-tool-lint.sh` | Lint pós-edição (fmt + clippy) |
| `.claude/hooks/stop-dod.sh` | Validação de Definition of Done |
| `CLAUDE.md` | Este arquivo — documentação das regras |
| `OPENCLAUDE.md` | Diferenças de comportamento OpenClaude vs Claude Code |
