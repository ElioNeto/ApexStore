# Comando: /debug-performance

Guia para investigar problemas de performance no ApexStore.

## Benchmarks disponíveis

```bash
cargo bench                         # roda todos os benchmarks (criterion)
cargo bench -- <nome_do_bench>      # bench específico
```

## Métricas em runtime

A API expõe telemetria completa:
```bash
curl http://localhost:8080/stats/all | jq
```

Seções do response:
- `memory` — tamanho da MemTable, contagem de chaves
- `wal` — contagem de entradas, tamanho do arquivo
- `disk` — bytes em disco, número de SSTables
- `bloom` — taxa de false positives
- `cache` — hit rate do Block Cache

## Gargalos comuns

| Sintoma | Causa provável | Onde olhar |
|---|---|---|
| Writes lentos | `WAL_SYNC_MODE=fsync` com I/O lento | `storage/wal.rs` + env config |
| Reads lentos | Block Cache com hit rate baixo | `storage/cache.rs`, `CACHE_SIZE` |
| Flush frequente | `MEMTABLE_MAX_SIZE` muito pequeno | `infra/config.rs` |
| Bloom false positives altos | Muitas SSTables, tamanho do filtro | `storage/builder.rs` |
| SSTable reads lentos | Sparse index muito esparso | `storage/reader.rs` |

## Variáveis de tuning

Ver `.env.example` para valores default e limites. Ajuste por ordem de impacto:
1. `MEMTABLE_MAX_SIZE` — maior = menos flushes = mais RAM
2. `BLOCK_CACHE_SIZE` — maior = mais hits = mais RAM
3. `WAL_SYNC_MODE=none` — elimina fsync, risco de perda de dados
4. `BLOOM_FILTER_FP_RATE` — menor = menos false positives = mais memória
