# /pr — Abrir Pull Request

Gera um PR completo para a branch atual seguindo o padrão do ApexStore.

## O que fazer

1. Rode `git log main..HEAD --oneline` para listar os commits da branch
2. Rode `git diff main...HEAD --stat` para ver os arquivos alterados
3. Leia `.claude/pr-checklist.md` para aplicar o checklist
4. Monte o corpo do PR com:
   - **Título**: `tipo(escopo): descrição curta` (Conventional Commits)
   - **Motivação**: por que essa mudança existe
   - **O que mudou**: lista dos principais arquivos/módulos
   - **Como testar**: comandos `cargo test` ou `curl` para validar
   - **Known limitations** se houver
   - Checklist de DoD (formato checkbox)
5. Exiba o rascunho do PR para aprovação antes de criar

## Leia também

- `.claude/skills/rust-lsm.md` — convenções Rust do projeto
- `.claude/pr-checklist.md` — checklist obrigatório
- `.claude/decisions.md` — decisões de arquitetura já tomadas
