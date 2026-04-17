# ApexStore — Project Intelligence

## O que é este projeto

ApexStore é uma **storage engine LSM-Tree** escrita em Rust, com REST API (Actix-Web), CLI REPL, TUI (Ratatui) e um frontend Angular 17. O projeto é um monorepo com backend Rust na raiz e frontend em `frontend/`.

## Stack completa

| Camada | Tecnologia |
|---|---|
| Storage Engine | Rust 2021, LSM-Tree, SSTable V2, LZ4, Bloom Filter |
| API Server | Actix-Web 4, Tokio, actix-cors |
| CLI | REPL interativo (`src/cli/`) |
| TUI | Ratatui + Crossterm (`src/bin/tui.rs`) |
| Frontend | Angular 17, Signals, standalone components, SCSS |
| Serialização | Bincode (binário) + Serde JSON (API) |
| Observabilidade | Tracing + tracing-subscriber |
| Auth | SHA2 + Base64 + actix-web-httpauth |
| Concorrência | parking_lot (RwLock/Mutex), Rayon |

## Arquitetura em camadas

```
src/
├── core/           # Domínio puro — Engine, MemTable, LogRecord
│   ├── engine.rs   # LSM Engine central (put/get/flush/recovery)
│   ├── memtable.rs # BTreeMap in-memory com size tracking
│   └── log_record.rs # Modelo de dados de entrada
├── storage/        # Persistência — WAL, SSTable V2, Block, Cache
│   ├── wal.rs      # Write-Ahead Log (ACID durability)
│   ├── reader.rs   # SSTableManager (leitura, busca, Bloom Filter)
│   ├── builder.rs  # SSTableBuilder (escrita com LZ4 + Sparse Index)
│   ├── block.rs    # Leitura/escrita de blocos de dados
│   ├── cache.rs    # Block Cache LRU global
│   ├── iterator.rs # Iteradores de range/prefix
│   └── sst_iterator.rs # Iterator sobre SSTables
├── infra/          # Codec, Error, Config (env vars)
├── api/            # Handlers Actix-Web (REST)
├── cli/            # REPL implementation
├── features/       # Feature flags runtime
├── bin/
│   ├── server.rs   # Entrypoint HTTP server
│   ├── cli.rs      # Entrypoint REPL
│   └── tui.rs      # Entrypoint TUI
frontend/
└── src/app/
    ├── pages/      # dashboard, key-explorer, stats
    ├── components/ # toast, stat-card
    └── services/   # ApexStoreService, ToastService
```

## Fluxo de escrita (crítico)

```
put(key, value)
  → WAL.append()          # durabilidade primeiro
  → MemTable.insert()     # BTreeMap in-memory
  → if memtable.is_full()
      → SSTableBuilder.build()  # flush para disco
      → MemTable.clear()
```

## Fluxo de leitura

```
get(key)
  → MemTable.get()        # 1º: mais rápido (~1.2M ops/s)
  → BlockCache.get()      # 2º: LRU cache
  → SSTableManager        # 3º: Bloom Filter → Sparse Index → Block read
```

## Variáveis de ambiente chave

Ver `.env.example` para lista completa. As principais:
- `MEMTABLE_MAX_SIZE` — tamanho máximo antes do flush (default 16MB)
- `WAL_SYNC_MODE` — `fsync` | `none` (tradeoff durabilidade vs throughput)
- `DATA_DIR` — diretório de dados (SSTables + WAL)
- `SERVER_PORT` — porta do servidor HTTP (default 8080)
- `AUTH_ENABLED` — habilita autenticação Bearer

## REST API endpoints

| Method | Path | Body / Response |
|---|---|---|
| POST | `/keys` | `{"key": "k", "value": "v"}` |
| GET | `/keys/{key}` | `{"value": "v"}` |
| GET | `/stats/all` | JSON com sections: memory, wal, disk, bloom, cache |

## Frontend (Angular 17)

- Roda em `http://localhost:4200`
- API base configurada em `frontend/src/environments/environment.ts`
- Usa `signal()`, `input()`, `@if`, `@for` (zero NgModules)
- 3 páginas: Dashboard, Key Explorer, Statistics

## Convenções de código Rust

- **SOLID estrito**: cada struct tem responsabilidade única
- Erros com `thiserror` — nunca `.unwrap()` em produção
- Locks com `parking_lot::RwLock` (não `std::sync`)
- Logs com `tracing::` macros (não `println!`)
- Testes de integração em `tests/`, unit tests inline com `#[cfg(test)]`
- Benchmarks com `criterion` em `benches/`

## Convenções Angular

- Todos os componentes são **standalone**
- Estado reativo exclusivamente com **Signals** (`signal`, `computed`, `input`)
- Template syntax nova: `@if`, `@for` (nunca `*ngIf`, `*ngFor`)
- Injeção com `inject()` (nunca no constructor)
- SCSS com variáveis CSS custom properties (ver `styles.scss`)

## Workflow de desenvolvimento

```bash
# Backend
cargo run --release --bin apexstore-server  # API em :8080
cargo run --release --bin apexstore-cli     # REPL
cargo run --release --bin apexstore-tui     # TUI
cargo test                                   # testes
cargo clippy -- -D warnings                  # lint

# Frontend
cd frontend && npm install && npm start      # Angular em :4200
```

## CI/CD

- Trunk-based development: branches de feature → PR → merge em `main`
- CI valida: `cargo fmt`, `cargo clippy`, `cargo test`, `cargo build`
- Merge em `main` → auto-bump de versão no `Cargo.toml` → tag → GitHub Release
- Ver `.github/workflows/` para detalhes

## Roadmap ativo

- `v2.2` — Storage iterators para range queries (em desenvolvimento)
- `v2.3` — Concurrent read optimization  
- `v3.0` — Leveled/Tiered Compaction Strategies
