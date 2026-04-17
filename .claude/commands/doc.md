# /doc — Gerar Documentação

Gera ou atualiza documentação para um módulo ou função.

## O que fazer

1. Se `$ARGUMENTS` for um caminho de arquivo (`src/core/engine.rs`), documente todas as funções públicas (`pub fn`) sem `///` ou com doc incompleto
2. Se for um nome de módulo (`storage::wal`), documente o módulo inteiro
3. Padrão de doc Rust a seguir:

```rust
/// Descrição de uma linha do que a função faz.
///
/// # Arguments
/// * `key` - Descrição do argumento
///
/// # Returns
/// Descrição do retorno
///
/// # Errors
/// Lista os casos de `Err(...)` possíveis
///
/// # Example
/// ```
/// // exemplo mínimo compilável
/// ```
```

4. NÃO altere a lógica — só adicione/corrija comentários
5. Exiba o diff para aprovação antes de aplicar

## Leia também

- `.claude/skills/rust-lsm.md`
