# Catálogo de Erros — ApexStore

Todos os erros do sistema estão em `src/infra/error.rs`.
Use este catálogo para mapear erros para respostas HTTP e mensagens de log.

## `ApexError` — tipo principal

| Variante | Causa | HTTP | Log Level |
|---|---|---|---|
| `KeyNotFound` | Chave não existe em nenhuma camada | 404 | `debug` |
| `KeyEmpty` | String de chave vazia | 400 | `warn` |
| `KeyTooLong` | Chave > limite configurado | 400 | `warn` |
| `ValueTooLarge` | Value > `MAX_RAW_PAYLOAD_SIZE` | 400 | `warn` |
| `MemTableFull` | MemTable no limite, flush falhou | 503 | `error` |
| `FlushError(msg)` | Falha ao escrever SSTable | 500 | `error` |
| `WalError(msg)` | Falha no Write-Ahead Log | 500 | `error` |
| `IoError(err)` | Erro de I/O genérico | 500 | `error` |
| `CodecError(msg)` | Falha em serialização bincode | 500 | `error` |
| `CompressionError` | Falha no LZ4 | 500 | `error` |
| `CorruptedData(msg)` | Checksum inválido, dado corrompido | 500 | `error` |
| `ConfigError(msg)` | Variável de ambiente inválida | — (fatal, startup) | `error` |
| `AuthError` | Token inválido ou ausente | 401 | `warn` |
| `FeatureDisabled` | Feature flag desativada | 403 | `info` |

## Como adicionar novo erro

1. Adicionar variante em `src/infra/error.rs`:
```rust
#[derive(Debug, thiserror::Error)]
pub enum ApexError {
    // ...
    #[error("nova mensagem: {0}")]
    NovoErro(String),
}
```

2. Mapear no handler HTTP em `src/api/`:
```rust
Err(ApexError::NovoErro(msg)) => {
    HttpResponse::UnprocessableEntity()
        .json(serde_json::json!({ "error": msg }))
}
```

3. Documentar nesta tabela.

## Erros de recovery (startup)

Durante `LsmEngine::new()`, erros de recovery são tratados assim:
- WAL corrompido parcialmente → replay até o último registro válido (CRC32), log `warn`
- SSTable corrompido → ignora o arquivo, log `error`, continua com os demais
- Nenhum dado é silenciosamente perdido sem log

## Erros do Frontend

O `ApexStoreService` repassa o `err.error.message` do response body.
O `ToastService` categoriza:
- HTTP 4xx → `toast.error()` — problema do usuário
- HTTP 5xx → `toast.error()` — problema do servidor
- Network error → `toast.error('Could not connect to API')`
