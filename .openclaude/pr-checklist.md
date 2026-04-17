# Checklist de PR — ApexStore

Use este arquivo como referência antes de abrir ou revisar um Pull Request.
O CI bloqueia merge se qualquer item de CI falhar.

## Checklist do autor

### Geral
- [ ] O PR tem uma responsabilidade única e clara
- [ ] O título segue Conventional Commits: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`
- [ ] Sem arquivos de debug esquecidos (`.env`, `data/`, arquivos `*.sst`/`*.log`)
- [ ] `CHANGELOG.md` atualizado se for breaking change ou feature

### Código Rust
- [ ] `cargo fmt --check` passa
- [ ] `cargo clippy -- -D warnings` passa sem supressões duvidosas (`#[allow(...)]`)
- [ ] `cargo test` passa (unit + integração)
- [ ] `cargo build --release` compila
- [ ] Sem `.unwrap()` fora de `#[cfg(test)]`
- [ ] Sem `println!` / `eprintln!` — usar `tracing::`
- [ ] Sem `unsafe` sem comentário `// SAFETY: ...` justificando
- [ ] Novos erros adicionados ao `error-catalog.md`
- [ ] Novas env vars documentadas em `.env.example` e `config-reference.md`
- [ ] Testes adicionados para o comportamento novo

### API (se alterou `src/api/`)
- [ ] Novo endpoint documentado em `CLAUDE.md` (tabela REST API)
- [ ] Response segue padrão: sucesso com JSON, erro com `{"error": "msg"}`
- [ ] CORS não foi alterado por handler (apenas global)
- [ ] Auth: novas rotas protegidas se necessário

### Frontend (se alterou `frontend/`)
- [ ] `npm run build` passa sem erros de TypeScript
- [ ] Sem `*ngIf` / `*ngFor` — usar `@if` / `@for`
- [ ] Sem `NgModule` novos
- [ ] Sem `HttpClient` injetado diretamente em componente
- [ ] Signals usados para todo estado mutante
- [ ] Nova página adicionada ao `app.routes.ts` e `navItems`

### Storage Engine (se alterou `src/core/` ou `src/storage/`)
- [ ] Invariantes do LSM-Tree preservados (ver `skills/lsm-tree-concepts.md`)
- [ ] WAL sempre escrito antes da MemTable
- [ ] Nenhum `RwLock` write segurado durante I/O de disco
- [ ] Teste de restart/recovery adicionado ou verificado
- [ ] Bloom Filter e Sparse Index atualizados se mudou formato SSTable

## Checklist do revisor

- [ ] A lógica faz sentido sem precisar rodar o código
- [ ] Sem vazação de abstrações (ex: `storage/` importando de `api/`)
- [ ] Dependências novas justificadas (`Cargo.toml`)
- [ ] Performance: sem alocações desnecessárias em hot paths
- [ ] Sem lock contention óbvia (write lock em operação lenta)

## O que o CI verifica automaticamente

```
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```

Merge em `main` → auto-bump de `patch` no `Cargo.toml` → tag + GitHub Release.
