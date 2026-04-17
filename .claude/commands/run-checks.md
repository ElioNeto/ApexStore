# Comando: /run-checks

Verifica a qualidade do código antes de abrir um PR.

## Sequência obrigatória (Rust)

```bash
# 1. Formatação
cargo fmt --check

# 2. Lint (zero warnings)
cargo clippy -- -D warnings

# 3. Testes
cargo test

# 4. Build release
cargo build --release
```

## Frontend

```bash
cd frontend
npm run build
```

## O que verificar manualmente

- [ ] Sem `println!` ou `dbg!` esquecidos (usar `tracing::`)
- [ ] Sem `.unwrap()` em código fora de `#[cfg(test)]`
- [ ] Sem `unsafe` não documentado
- [ ] Variáveis de ambiente novas documentadas no `.env.example`
- [ ] Breaking changes documentados em `CHANGELOG.md`
- [ ] Frontend: sem `*ngIf`/`*ngFor` — usar `@if`/`@for`
- [ ] Frontend: sem componentes com `NgModule`

## Antes do merge

O CI roda automaticamente no PR. Mas rodar localmente poupa tempo de ciclo.
