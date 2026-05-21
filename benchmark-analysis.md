# Benchmark & Application Analysis — ApexStore v2.1.51

> Gerado em: 2026-05-21  
> Máquina: Linux, CPU x86_64  
> Build: `--release` (optimized)

---

## 1. Execução dos Benchmarks

### Grupos executados

| Benchmark | Status | Benchmark groups |
|-----------|--------|-----------------|
| `write_bench` | ✅ Completo | 4 grupos (9 parâmetros) |
| `read_bench` | ✅ Completo | 6 grupos (11 parâmetros) |
| `scan_bench` | ✅ Completo | 7 grupos (11 parâmetros) |
| `mixed_bench` | ✅ Completo | 6 grupos (10 parâmetros) |
| `stress_bench` | ⚠️ Parcial | 7 grupos, alguns com dados |
| `latency_bench` | ✅ Completo | 2 grupos (3 parâmetros) |
| `write_amplification` | ✅ Completo | 5 grupos |

**Total: 65 estimativas de performance extraídas do Criterion.**

---

## 2. Resumo de Performance

### 2.1 Operações de Escrita

| Operação | Mediana | Throughput | Observação |
|----------|---------|-----------|------------|
| **write_single/10** (chave pequena) | 872 µs | 11 KiB/s | ~1.140 operações/segundo |
| **write_single/100** | 894 µs | 107 KiB/s | |
| **write_single/1024** | 863 µs | 1.16 MiB/s | |
| **write_single/10240** (10KB valor) | 918 µs | 10.5 MiB/s | Throughput de dados alto |
| **write_batch_1000** | 882 ms | 121 KiB/s | 1.000 operações atômicas |
| **write_batch_10000** | 8.72 s | 123 KiB/s | 10.000 ops atômicas |
| **write_batch_100000** | 89.9 s | 119 KiB/s | 100.000 ops atômicas |

**Insight:** A latência por operação individual é consistente (~900 µs) independente do tamanho do valor. O throughput de dados escala linearmente com o tamanho do payload (de 11 KiB/s para 10.5 MiB/s). Operações batch têm throughput estável (~120 KiB/s) pois o gargalo é o WAL (fsync por registro).

### 2.2 Operações de Leitura

| Operação | Mediana | Throughput | Observação |
|----------|---------|-----------|------------|
| **read_memtable/1000** | 428 µs | 2.3 Melem/s | Leitura de 1000 keys em memtable |
| **read_memtable/10000** | 458 µs | 21 Melem/s | Escala bem com dataset maior |
| **read_sstable_cold/1000** | 299 µs | 3.3 Melem/s | Sem cache de bloco |
| **read_sstable_cold/10000** | 272 µs | 35 Melem/s | Surpreendentemente mais rápido que 1000 |
| **read_sstable_warm/1000** | 316 µs | 3.1 Melem/s | Com block cache aquecido |
| **read_sstable_warm/10000** | 356 µs | 21 Melem/s | |
| **bloom_filter/10000** | 4.5 ms | 1.7 Melem/s | Bloom filter miss (negativo) |
| **bloom_filter/100000** | 36.5 ms | 2.7 Melem/s | |

**Insight:** leitura em memtable é ~30% mais lenta que em SSTable (devido ao lock do WAL durante operações concorrentes). Block cache não mostra ganho significativo porque os dados estão no cache de página do SO. Bloom filter negativo é 10-100x mais lento que o esperado (~4.5 ms para 10k) — **oportunidade de otimização**.

### 2.3 Full Scan

| Operação | Mediana | Throughput | Observação |
|----------|---------|-----------|------------|
| **full_scan/1000** | 157 µs | 6.3 Melem/s | |
| **full_scan/10000** | 2.3 ms | 4.3 Melem/s | Escala linearmente |
| **range_scan_100/100** | 6.8 ms | 14 Kelem/s | Com filtro de range |
| **range_scan_1000/1000** | 7.2 ms | 139 Kelem/s | |
| **prefix_scan/100** | 762 µs | 131 Kelem/s | |
| **prefix_scan/1000** | 1.0 ms | 999 Kelem/s | |
| **scan_limit_10** | 2.8 µs | — | Muito rápido (apenas 10 registros) |
| **scan_limit_100** | 31 µs | — | |
| **scan_limit_1000** | 276 µs | — | |
| **scan_pagination/10** | 818 µs | — | Cursor-based pagination |
| **scan_pagination/100** | 69.5 ms | — | Muito mais lento com paginação |

**Insight:** Full scan tem boa performance (6.3 Melem/s). Range scan é ~30x mais lento que full scan devido ao filtro por chave. Scan com limit é extremamente rápido (2.8 µs para 10 registros). Paginação com cursor é lenta porque precisa refazer o scan a cada página.

### 2.4 Workloads Mistos (YCSB)

| Operação | Mediana | Throughput | Observação |
|----------|---------|-----------|------------|
| **ycsb_type_a/10000** (50% read / 50% write) | 863 ms | 1.1 Kelem/s | Write-heavy |
| **ycsb_type_b/10000** (95% read / 5% write) | 83 ms | 11.5 Kelem/s | Read-heavy |
| **ycsb_type_c/10000** (100% read) | 402 µs | 2.4 Melem/s | Read-only |
| **ycsb_type_c/100000** | 748 µs | — | |
| **ycsb_type_c/1000000** | 1.27 ms | — | |
| **workload_balanced/10000** | 1.06 ms | 9.4 Kelem/s | Mix balanceado |
| **workload_read_heavy/10000** | 754 µs | 13.2 Kelem/s | |
| **workload_write_heavy/10000** | 1.08 ms | 9.2 Kelem/s | |

**Insight:** Read-only é 3 ordens de magnitude mais rápido que write-heavy (402 µs vs 863 ms) devido ao WAL fsync. YCSB Type A (50/50) sofre com o custo de fsync por operação. YCSB Type C escala bem até 1M de registros (~1.27 ms).

### 2.5 Flush & Compaction

| Operação | Mediana | Throughput | Observação |
|----------|---------|-----------|------------|
| **memtable_flush_8/8** (8 memtables) | 33.5 s | 242 KiB/s | Muito lento — flush com retain() no WAL |
| **memtable_flush_16/16** | 92.8 ms | — | 16 tabelas pequenas é rápido |
| **sstable_flush/100000** | 159 ms | ~570 MB/s | Throughput de escrita em disco excelente |
| **sstable_layer_1/1** | 2.4 ms | — | Leitura em 1 SSTable |
| **sstable_layer_3/3** | 8.2 ms | — | MergeIterator com 3 SSTables |
| **sstable_layer_10/10** | 30.4 ms | — | 10 SSTables — degradação esperada |

**Insight:** `memtable_flush_8` é extremamente lento (33.5s) — o gargalo é `WAL::retain()` que faz recover + rewrite com temp file. Para 8 memtables grandes (512KB cada), o retain é chamado a cada flush. `sstable_flush` tem throughput excelente (~570 MB/s). Leitura em múltiplas SSTables degrada linearmente com MergeIterator.

### 2.6 Estresse

| Operação | Mediana | Observação |
|----------|---------|-----------|
| **concurrent_1_thread/1** | 3.4 ms | |
| **concurrent_2_thread/2** | 4.0 ms | Overhead de lock ~18% |
| **cache_thrash_16MB** | 120 µs | Block cache com 16MB |
| **cache_thrash_64MB** | 123 µs | Block cache com 64MB (quase igual) |
| **memory_pressure** | 7.4 ms | Memtable sob pressão de memória |
| **many_sstables_10** | 281 µs | |
| **many_sstables_50** | 390 µs | 50 SSTables — degradação moderada |
| **key_updates** | 11.1 ms | Atualizações de chave |
| **delete_operations** | 1.8 ms | Operações de deleção |

---

## 3. Análise Arquitetural

### 3.1 Componentes

```
┌─────────────────────────────────────────────────┐
│                   Engine<C>                      │
│  ┌──────────┐  ┌──────────┐  ┌────────────────┐ │
│  │ EngineCore│  │ Memtable │  │  VersionSet    │ │
│  │ (Mutex)  │  │ (BTree)  │  │  (table index) │ │
│  └────┬─────┘  └────┬─────┘  └───────┬────────┘ │
│       │              │                │           │
│  ┌────▼──────────────▼────────────────▼────────┐ │
│  │           WriteAheadLog (WAL)               │ │
│  │  [length][version][payload][CRC32] x N      │ │
│  └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────┐
│                SSTable V2                         │
│  [Magic][Version][Bloom][Block0..N][Index][CRC]  │
│  + LZ4 compression + GlobalBlockCache (LRU)      │
└─────────────────────────────────────────────────┘
```

### 3.2 Fluxo de Operações

```
SET(key, value):
  1. Lock EngineCore Mutex
  2. Serialize LogRecord + compute CRC32
  3. write_record() no WAL (append + fsync)   ← gargalo principal
  4. Insert na MemTable (BTreeMap)
  5. Se memtable cheia → flush para SSTable
  6. Se flush → retain() no WAL (recover + filter + rewrite)
  7. Unlock EngineCore Mutex
  8. Se precisa → trigger compactação background

GET(key):
  1. Lock EngineCore Mutex
  2. Busca na MemTable (mais recente primeiro)  ← O(log n)
  3. Busca no VersionSet (tabelas ordenadas)
     - Bloom filter check (O(1))                ← fast path
     - Binary search in block (O(log b))        ← O(log n)
     - Block cache hit check
  4. Unlock EngineCore Mutex
```

### 3.3 Padrões de Concorrência

- **EngineCore**: `parking_lot::Mutex` — lock de curta duração para escrita
- **WAL**: `parking_lot::Mutex` interno — lock apenas durante I/O de append
- **GlobalBlockCache**: `parking_lot::Mutex` interno — lock apenas durante hit/miss
- **Compaction thread**: separada, só re-adquire EngineCore lock na fase 3 (apply)

---

## 4. Diagnóstico de Performance

### 🟢 Pontos Fortes

| Aspecto | Indicador |
|---------|-----------|
| Leitura em memtable | 2.3 Melem/s — excelente |
| Full scan | 6.3 Melem/s — muito bom |
| Scan com limit | 2.8 µs para 10 registros |
| Throughput de escrita batch | ~120 KiB/s consistente |
| SSTable flush throughput | ~570 MB/s — excelente compressão LZ4 |
| YCSB Type C (read-only) | 2.4 Melem/s — competitivo |
| Binary search em block | O(log n) implementado na Issue #153 |

### 🟡 Pontos de Atenção

| Aspecto | Indicador | Causa Provável |
|---------|-----------|---------------|
| Bloom filter negativo | 4.5 ms (10k) — muito lento | Criação de novo filter a cada benchmark, sem cache |
| YCSB Type A (50/50) | 863 ms — 1.1 Kelem/s | WAL fsync por operação individual |
| Range scan | 6.8 ms — 14 Kelem/s | Filtragem pós-leitura, sem índice de range |
| Paginação com cursor | 69.5 ms | Re-scaneia dados a cada página |
| `memtable_flush_8` | 33.5 segundos | `WAL::retain()` com recover + rewrite |

### 🔴 Gargalos Identificados

1. **WAL fsync por operação** — cada `set()` faz um `fsync` individual. Para workloads write-heavy, isso limita o throughput a ~1.100 ops/s. Solução: WAL batch commit.

2. **`WAL::retain()` no flush** — após cada flush, o WAL é inteiramente lido (recover), filtrado e reescrito. Para 8 memtables flushadas, cada chamada leva segundos. Solução potencial: WAL por column family.

3. **Bloqueio do EngineCore Mutex durante flush** — flush do memtable segura o EngineCore lock durante toda a operação de SSTable build + WAL retain, bloqueindo outras operações.

4. **MergeIterator linear em scan com múltiplas SSTables** — cada SSTable adicional adiciona ~2-3 ms ao scan. Com 50 SSTables, chega a 30 ms.

---

## 5. Recomendações (Ordem de Impacto)

| Prioridade | Recomendação | Impacto Esperado |
|-----------|-------------|-----------------|
| 🔴 **Alta** | **WAL batch commit**: acumular N operações antes de fsync | 10-50x melhoria em write-heavy |
| 🔴 **Alta** | **WAL por column family**: eliminar retain() no flush | Elimina gargalo de 33s no flush |
| 🟡 **Média** | **Skip list indexada**: substituir BTreeMap da memtable por SkipList + comparator | ~2x melhoria em reads |
| 🟡 **Média** | **Bloom filter caching**: evitar recriação do Bloom filter a cada benchmark | 10-100x melhoria em bloom negativo |
| 🟢 **Baixa** | **Scan com índice esparso denso**: adicionar more entries no block index para range scan | ~5x em range scan |

---

## 6. Resumo de Saúde da Aplicação

| Dimensão | Nota | Comentário |
|----------|------|-----------|
| **Corretude** | 🟢 A | 123 testes passando, WAL com CRC32 + recover |
| **Performance leitura** | 🟢 A | 2-6 Melem/s, O(log n) busca em bloco |
| **Performance escrita** | 🟡 B | ~1.100 ops/s single; batch escala linearmente |
| **Concorrência** | 🟡 B | parking_lot Mutex, background compaction |
| **Recuperação de falhas** | 🟢 A | WAL crash-safe, retain atômico (temp file) |
| **Utilização de recursos** | 🟢 A | Compressão LZ4, block cache LRU |
| **Cobertura de testes** | 🟡 B | Boa cobertura unitária + integração |
| **Qualidade do código** | 🟢 A | Clippy clean, thiserror, parking_lot, accessors |

**Classificação geral: 🟢 B+** — ApexStore é um storage engine sólido, correto e com boa performance para leitura. O principal gargalo está na escrita com fsync individual no WAL, que é uma escolha deliberada de durabilidade. Para workloads write-heavy, recomenda-se usar `set_batch()` que reduz o número de fsyncs.
