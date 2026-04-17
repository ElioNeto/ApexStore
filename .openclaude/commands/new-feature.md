# Comando: /new-feature

Cria uma nova feature seguindo os padrões do ApexStore.

## Uso
```
/new-feature <nome-da-feature> [camada: core|storage|api|frontend]
```

## O que fazer

### Se camada = `core` ou `storage` (Rust)

1. Criar o arquivo em `src/<camada>/<nome>.rs`
2. Expor no `mod.rs` da camada
3. Seguir o padrão:
   - Struct com responsabilidade única
   - Errors com `thiserror`
   - Locks com `parking_lot`
   - Logs com `tracing::`
   - `#[cfg(test)]` com pelo menos 1 teste unitário
4. Se a feature tocar o Engine, atualizar `src/core/engine.rs`
5. Criar teste de integração em `tests/`

### Se camada = `api` (Rust + Actix)

1. Criar handler em `src/api/<nome>_handler.rs`
2. Registrar rota em `src/api/mod.rs` ou `src/api/routes.rs`
3. Body/response tipados com `serde::Deserialize/Serialize`
4. Retornar `actix_web::Result` com erros mapeados

### Se camada = `frontend` (Angular)

1. Criar componente em `frontend/src/app/pages/<nome>/` ou `components/<nome>/`
2. Componente standalone com `signal()` para estado
3. Usar `@if` / `@for` no template (nunca `*ngIf` / `*ngFor`)
4. Injetar dependências com `inject()` no corpo da classe
5. Adicionar rota em `frontend/src/app/app.routes.ts` se for página
6. Adicionar item de navegação em `AppComponent.navItems` se necessário

## Checklist
- [ ] Código segue as convenções do `CLAUDE.md`
- [ ] Sem `.unwrap()` em código de produção (Rust)
- [ ] Sem `NgModules` novos (Angular)
- [ ] Testes adicionados
- [ ] `cargo clippy` passa sem warnings
