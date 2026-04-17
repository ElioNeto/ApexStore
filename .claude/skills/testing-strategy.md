# Skill: Estratégia de Testes — ApexStore

Use esta skill ao escrever ou corrigir testes.

## Tipos de teste no projeto

| Tipo | Localização | Velocidade | O que cobre |
|---|---|---|---|
| Unit | `src/**` com `#[cfg(test)]` | Rápido | Lógica de uma struct isolada |
| Integração | `tests/` | Médio | Fluxo end-to-end com disco real |
| Benchmark | `benches/` | Lento | Throughput e latência |

## Unit tests — padrão

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, MinhaStruct) {
        let dir = TempDir::new().expect("failed to create tempdir");
        let s = MinhaStruct::new(dir.path());
        (dir, s)  // retornar TempDir para manter o diretório vivo
    }

    #[test]
    fn test_insert_and_retrieve() {
        let (_dir, mut s) = setup();
        s.put("key1", "value1").unwrap();
        assert_eq!(s.get("key1").unwrap(), Some("value1".to_string()));
    }

    #[test]
    fn test_missing_key_returns_none() {
        let (_dir, s) = setup();
        assert_eq!(s.get("nao_existe").unwrap(), None);
    }

    #[test]
    fn test_overwrite_key() {
        let (_dir, mut s) = setup();
        s.put("k", "v1").unwrap();
        s.put("k", "v2").unwrap();
        assert_eq!(s.get("k").unwrap(), Some("v2".to_string()));
    }
}
```

## Testes de integração — padrão

Em `tests/<nome>.rs`:

```rust
use apexstore::{LsmEngine, Config};
use tempfile::TempDir;

fn engine_for_test() -> (TempDir, LsmEngine) {
    let dir = TempDir::new().unwrap();
    let config = Config::test_defaults(dir.path());
    let engine = LsmEngine::new(&config).unwrap();
    (dir, engine)
}

#[test]
fn test_persistence_across_restart() {
    let dir = TempDir::new().unwrap();
    let config = Config::test_defaults(dir.path());

    // Sessão 1: escreve
    {
        let engine = LsmEngine::new(&config).unwrap();
        engine.put("persistent_key", "value").unwrap();
        engine.flush().unwrap();
    } // engine dropado, simula shutdown

    // Sessão 2: recovers e lê
    {
        let engine = LsmEngine::new(&config).unwrap(); // WAL replay aqui
        assert_eq!(
            engine.get("persistent_key").unwrap(),
            Some("value".to_string())
        );
    }
}
```

## Cenários obrigatórios ao mexer no Engine

- [ ] `put` → `get` retorna o valor
- [ ] `put` → overwrite → `get` retorna novo valor
- [ ] `get` de chave inexistente retorna `None`
- [ ] `put` → flush → `get` (leitura de SSTable)
- [ ] Restart (drop + new) → WAL recovery → `get` retorna valor
- [ ] N puts até MemTable cheia → flush automático → `get` de todas as chaves

## Cenários para WAL

- [ ] Arquivo WAL criado na primeira escrita
- [ ] Após replay, todas as chaves são recuperadas
- [ ] WAL corrompido (truncado) → erro controlado, não panic

## Cenários para SSTable

- [ ] Bloom filter filtra chaves ausentes (zero false negatives)
- [ ] Chave no limite de bloco é encontrada corretamente
- [ ] SSTable com LZ4 comprimido é lido corretamente
- [ ] Sparse index aponta para o bloco correto

## Benchmarks

```rust
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use apexstore::{LsmEngine, Config};
use tempfile::TempDir;

fn bench_sequential_writes(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let config = Config::test_defaults(dir.path());
    let engine = LsmEngine::new(&config).unwrap();
    let mut i = 0u64;

    c.bench_function("sequential_put", |b| {
        b.iter(|| {
            engine.put(&format!("key:{i}"), "value").unwrap();
            i += 1;
        })
    });
}

criterion_group!(benches, bench_sequential_writes);
criterion_main!(benches);
```

## Comandos

```bash
cargo test                                # todos os testes
cargo test test_persistence              # teste específico
cargo test -- --nocapture               # ver println! nos testes
cargo bench                              # benchmarks
cargo bench -- bench_sequential_writes  # benchmark específico
```

## Anti-padrões a evitar

- ❌ Usar paths fixos (`/tmp/test`) — sempre `TempDir`
- ❌ Depender de ordem de execução entre testes
- ❌ Compartilhar estado global entre testes
- ❌ Testar implementação interna — testar comportamento observável
- ❌ Testes sem assertion (`assert!`, `assert_eq!`)
