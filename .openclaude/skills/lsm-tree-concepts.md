# Skill: Conceitos LSM-Tree — ApexStore

Use esta skill ao implementar ou modificar componentes de storage.

## O que é LSM-Tree

Log-Structured Merge-Tree é uma estrutura de dados otimizada para **write-heavy workloads**. Escritas vão primeiro para memória (MemTable), depois são persistidas em arquivos imutáveis (SSTables) em disco.

## Componentes e invariantes

### MemTable
- **Estrutura**: `BTreeMap<String, String>` (ordenado por chave)
- **Invariante**: Sempre tem os dados mais recentes
- **Limite**: `MEMTABLE_MAX_SIZE` (default 16MB)
- **Ao atingir limite**: flush atômico → novo SSTable → limpa MemTable
- **Arquivo**: `src/core/memtable.rs`

### WAL (Write-Ahead Log)
- **Regra**: WAL **sempre** antes da MemTable
- **Formato**: registros binários sequenciais (bincode)
- **Recovery**: ao iniciar, replay do WAL reconstrói MemTable
- **Sync modes**:
  - `fsync`: `O_SYNC` — cada write vai para disco. Seguro, ~100k ops/s
  - `none`: buffer do OS — risco de perda em crash. ~500k ops/s
- **Arquivo**: `src/storage/wal.rs`

### SSTable V2
- **Imutável**: nunca modificado após escrito
- **Formato em disco**:
  ```
  [Data Blocks] [Sparse Index] [Bloom Filter] [Footer]
  ```
- **Data Block**: N pares key-value comprimidos com LZ4
- **Sparse Index**: 1 entrada a cada N blocos (tradeoff RAM vs I/O)
- **Bloom Filter**: probabilístico — responde "definitivamente não" ou "talvez"
- **Footer**: offsets do index e bloom filter no arquivo
- **Arquivos**: `src/storage/builder.rs` (escrita), `src/storage/reader.rs` (leitura)

### Block Cache
- Cache LRU global de blocos descomprimidos
- Evita re-leitura de disco e re-descompressão
- **Arquivo**: `src/storage/cache.rs`
- **Config**: `BLOCK_CACHE_SIZE` (número de blocos)

## Algoritmo de leitura — ordem crítica

```
get(key):
  1. MemTable.get(key)          → O(log n), mais recente
  2. BlockCache.get(key)        → O(1), blocos quentes
  3. Para cada SSTable (mais novo → mais antigo):
     a. BloomFilter.check(key)  → se "não", pula SSTable inteiro
     b. SparseIndex.find(key)   → offset aproximado do bloco
     c. Block.read_decompress() → I/O + LZ4 decompress
     d. Block.scan(key)         → busca linear no bloco
     e. Cache.insert(block)     → guarda para próxima leitura
```

## Algoritmo de escrita

```
put(key, value):
  1. WAL.append(LogRecord)      → flush para disco (se fsync)
  2. MemTable.insert(key, value)
  3. if MemTable.size >= MAX_SIZE:
     a. SSTableBuilder.build(memtable.iter()) → novo .sst
     b. SSTableManager.add(new_sst)
     c. MemTable.clear()
     d. WAL.rotate()             → novo arquivo WAL
```

## Compaction (v3.0)

Ainda não implementado. SSTables acumulam sem merge. O ROADMAP prevê:
- **Tiered**: agrupa SSTables de tamanho similar em tiers
- **Leveled**: mantém SSTables por nível com garantia de não-overlap

Ao implementar, o ponto de entrada será `src/storage/reader.rs` (SSTableManager).

## Bloom Filter — como usar corretamente

```rust
// Ao construir SSTable — adiciona todas as chaves
let mut bloom = BloomFilter::with_rate(fp_rate, expected_keys);
for (key, _) in memtable.iter() {
    bloom.insert(key);
}

// Ao ler — sempre checar antes de ir ao disco
if !bloom.contains(key) {
    return Ok(None); // definitivamente não está neste SSTable
}
// se retornou true: pode ser false positive → continuar busca
```

Taxa de false positive (`BLOOM_FILTER_FP_RATE`) default ~1%. Menor = mais RAM.

## Formato binário (Codec)

`src/infra/codec.rs` encapsula bincode:

```rust
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ApexError>
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ApexError>
```

O `LogRecord` em `src/core/log_record.rs` é a unidade atômica de dado:
```rust
pub struct LogRecord {
    pub key: String,
    pub value: Option<String>,  // None = tombstone (deleção)
    pub timestamp: u64,
    pub checksum: u32,           // CRC32
}
```

## Tombstones (deleção)

Em LSM-Tree, deletar = inserir um tombstone (`value: None`). O dado físico só é removido na compaction. Ao ler:
- Se encontrar tombstone no MemTable ou SSTable mais recente → retornar `None`
- Não continuar buscando em SSTables mais antigos
