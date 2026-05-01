## Regras Rust

### Convenções
- `clippy` obrigatório: `cargo clippy -- -D warnings`
- `rustfmt` para formatação
- Sem `unwrap()` em código de produção; usar `?` ou tratamento explícito
- Prefer `thiserror` para erros de biblioteca, `anyhow` para binários
- Lifetimes explícitos quando necessário, não por padrão

### Testes
- Unit tests no mesmo arquivo com `#[cfg(test)]`
- Integration tests em `tests/`
- `cargo test --all-features`

### Build e ferramentas
```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo audit
```
