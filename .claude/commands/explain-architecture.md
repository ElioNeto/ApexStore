# Comando: /explain-architecture

Explica a arquitetura de uma camada específica do ApexStore.

## Uso
```
/explain-architecture [camada]
```

## Mapa de arquivos por responsabilidade

### LSM Engine (`src/core/engine.rs`)
Cérebro do sistema. Coordena MemTable, WAL e SSTableManager. Usa `parking_lot::RwLock` para acesso concorrente seguro. Expõe `put`, `get`, `delete`, `flush`, `stats`.

### MemTable (`src/core/memtable.rs`)
BTreeMap em memória. Ordenado por chave. Rastreia tamanho em bytes. Quando atinge `MEMTABLE_MAX_SIZE`, o Engine faz flush para SSTable.

### WAL (`src/storage/wal.rs`)
Write-Ahead Log. Toda escrita vai ao WAL **antes** da MemTable. Garante recuperação após crash. Modes: `fsync` (seguro) ou `none` (rápido).

### SSTableBuilder (`src/storage/builder.rs`)
Constrói arquivos SSTable V2. Organiza dados em blocos com LZ4, gera Sparse Index e footer. Salva Bloom Filter para lookup rápido.

### SSTableManager/Reader (`src/storage/reader.rs`)
Gerencia múltiplos arquivos SSTable. Na leitura: 1) consulta Bloom Filter, 2) usa Sparse Index para localizar bloco, 3) descomprime e escaneia bloco.

### Block Cache (`src/storage/cache.rs`)
Cache LRU global de blocos descomprimidos. Evita releitura de disco para chaves quentes.

### Iteradores (`src/storage/iterator.rs`, `sst_iterator.rs`)
Permitem varredura ordenada por prefixo ou range. Base para os iterators de v2.2.

### REST API (`src/api/`)
Handlers Actix-Web. Recebem `web::Data<Arc<RwLock<LsmEngine>>>` como estado compartilhado. Sem lógica de negócio — apenas delegam ao Engine.

### Frontend (`frontend/src/app/`)
Angular 17 SPA. `ApexStoreService` é o único ponto de contato com a API. Componentes são puros consumidores de Signals.
