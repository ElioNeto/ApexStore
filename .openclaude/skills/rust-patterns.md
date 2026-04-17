# Skill: Padrões Rust — ApexStore

Use esta skill ao escrever qualquer código Rust novo no projeto.

## Tratamento de erros

Sempre use `thiserror`. O tipo central é `ApexError` em `src/infra/error.rs`.

```rust
// ✅ Correto
pub fn get(&self, key: &str) -> Result<Option<String>, ApexError> {
    self.memtable
        .read()
        .get(key)
        .map_err(ApexError::MemTable)
}

// ❌ Proibido em produção
pub fn get(&self, key: &str) -> String {
    self.memtable.read().get(key).unwrap()
}
```

Nunca use `.unwrap()` fora de `#[cfg(test)]`. Use `.expect("mensagem descritiva")` apenas em inicialização de processo (main/server bootstrap).

## Concorrência

Use sempre `parking_lot` — nunca `std::sync`:

```rust
use parking_lot::{RwLock, Mutex};

// Estado compartilhado no Engine
pub struct LsmEngine {
    inner: Arc<RwLock<EngineInner>>,
}

// Leitura
let guard = self.inner.read();

// Escrita (libera o lock assim que o bloco termina)
{
    let mut guard = self.inner.write();
    guard.memtable.insert(key, value);
} // lock liberado aqui
```

Nunca segure um lock write por mais tempo do que o necessário. Nunca chame código async dentro de um guard de lock síncrono.

## Logging e observabilidade

Use `tracing::` — nunca `println!` ou `eprintln!` em código de produção:

```rust
use tracing::{debug, info, warn, error, instrument};

#[instrument(skip(self), fields(key = %key))]
pub fn put(&self, key: &str, value: &str) -> Result<(), ApexError> {
    debug!("writing key to memtable");
    // ...
    info!(bytes = value.len(), "key written");
    Ok(())
}
```

Níveis:
- `trace!` — loops internos, block reads
- `debug!` — operações individuais (put/get)
- `info!` — eventos de ciclo de vida (flush, recovery, server start)
- `warn!` — condições recuperáveis (bloom false positive, cache miss alto)
- `error!` — falhas não recuperáveis

## Serialização

- **Bincode** (`src/infra/codec.rs`) para dados em disco (WAL, SSTable)
- **Serde JSON** para payloads HTTP da API

```rust
// Disco — bincode
use crate::infra::codec::{encode, decode};
let bytes = encode(&record)?;
let record: LogRecord = decode(&bytes)?;

// API — serde_json via actix-web
#[derive(Serialize, Deserialize)]
pub struct KeyValueRequest {
    pub key: String,
    pub value: String,
}
```

## Estrutura de um módulo novo

Template para novo arquivo em `src/<camada>/<nome>.rs`:

```rust
//! Breve descrição do módulo.

use crate::infra::error::ApexError;
use parking_lot::RwLock;
use tracing::{debug, info};

/// Descrição da struct.
pub struct MinhaStruct {
    // campos
}

impl MinhaStruct {
    pub fn new(/* params */) -> Self {
        Self { /* ... */ }
    }

    pub fn operacao(&self) -> Result<(), ApexError> {
        debug!("executando operacao");
        // ...
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_operacao_basica() {
        // arrange
        // act
        // assert
    }
}
```

## Testes

```rust
// Diretório temporário para testes com disco
use tempfile::TempDir;
let dir = TempDir::new().unwrap();
let path = dir.path();

// Nunca hardcode paths em testes
// Nunca dependência de estado global
// Cada teste deve ser completamente isolado
```

Benchmarks com Criterion ficam em `benches/` e seguem o padrão:
```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_put(c: &mut Criterion) {
    c.bench_function("put_100k", |b| {
        b.iter(|| { /* ... */ })
    });
}

criterion_group!(benches, bench_put);
criterion_main!(benches);
```
