# Skill: Rust LSM-Tree (ApexStore)

Conhecimento especializado sobre a arquitetura e convenções do ApexStore.
Carregue este skill sempre que for ler, escrever ou revisar código Rust do projeto.

---

## Camadas e responsabilidades

| Camada | Módulo | Responsabilidade |
|---|---|---|
| Domínio | `src/core/` | Engine, MemTable, LogRecord — zero I/O |
| Persistência | `src/storage/` | WAL, SSTable V2, Block, Cache, Iteradores |
| Infra | `src/infra/` | Codec, Config, Error types |
| API | `src/api/` | Handlers Actix-Web, sem lógica de negócio |
| CLI | `src/cli/` | REPL, sem lógica de negócio |

**Regra de dependência:** `api/cli` → `core` → `storage` → `infra`. Nunca o inverso.

---

## Fluxos críticos

### Escrita
```
put(key, value)
  → WAL.append()           # durabilidade primeiro — NUNCA pule
  → MemTable.insert()      # BTreeMap in-memory
  → if memtable.is_full()  # threshold: MEMTABLE_MAX_SIZE
      → SSTableBuilder.build()   # LZ4 + Sparse Index
      → MemTable.clear()
```

### Leitura (ordem obrigatória)
```
get(key)
  1. MemTable.get()        # ~1.2M ops/s, prioridade máxima
  2. BlockCache.get()      # LRU global — evita I/O
  3. SSTableManager        # Bloom Filter → Sparse Index → Block
```

### Range scan (v2.2)
```
scan_range(start, end, limit, cursor)
  → MemTable: BTreeMap::range() — O(log n)
  → SSTables: sst.scan() filtrado em memória (limitação conhecida)
  → Merge + dedup por tombstone
  → Retorna Vec<(key, value)> + Option<next_cursor>
```

---

## Convenções obrigatórias

### Erros
```rust
// ✅ CORRETO
use thiserror::Error;
#[derive(Error, Debug)]
pub enum ApexError { ... }
fn foo() -> Result<T, ApexError> { ... }

// ❌ ERRADO
fn foo() -> Result<T, String> { ... }
panic!("algo deu errado");  // nunca em produção
result.unwrap();             // nunca em produção
```

### Locks
```rust
// ✅ CORRETO — parking_lot sempre
use parking_lot::{RwLock, Mutex};
let guard = self.memtable.read();

// ❌ ERRADO
use std::sync::RwLock;  // nunca std::sync
```

### Logs
```rust
// ✅ CORRETO
tracing::debug!(key = %key, "cache miss");
tracing::info!(bytes = buf.len(), "flush iniciado");

// ❌ ERRADO
println!("debug: {key}");
eprintln!("erro: {e}");
```

### Testes
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn nome_descreve_o_que_testa() {
        // arrange → act → assert
    }
}
```

---

## SSTable V2 — formato

```
[Header: magic(4) + version(1) + flags(1) + block_count(4)]
[Data Blocks: N × Block { entries(LZ4) }]
[Sparse Index: Vec<(key, block_offset)>]
[Bloom Filter: bitset serializado]
[Footer: index_offset(8) + bloom_offset(8) + checksum(4)]
```

---

## Limitações conhecidas (v2.2)

1. **SSTable range scan**: usa `sst.scan()` full + filtro em memória
2. **SCAN CLI pagination**: para na primeira página em edge cases
3. **Cursor validation**: assume cursores válidos sem verificação

---

## Performance baseline

| Operação | Target | Medido |
|---|---|---|
| put (MemTable) | > 1M ops/s | ~1.2M ops/s |
| get (cache hit) | < 1µs | ~800ns |
| get (SSTable) | < 5ms | ~2-3ms |
| flush (16MB) | < 500ms | ~300ms |

---

## Checklist rápido antes de commitar

- [ ] `cargo fmt --all` sem diff
- [ ] `cargo clippy -- -D warnings` sem erros
- [ ] Sem `.unwrap()` fora de `#[cfg(test)]`
- [ ] Sem `println!` fora de `#[cfg(test)]`
- [ ] Funções públicas novas têm `///`
- [ ] Novo comportamento tem pelo menos 1 teste
