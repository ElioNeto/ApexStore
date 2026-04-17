# Referência de Configuração — ApexStore

Todas as variáveis de ambiente lidas por `src/infra/config.rs`.
Template em `.env.example`.

## Servidor

| Variável | Default | Tipo | Descrição |
|---|---|---|---|
| `HOST` | `0.0.0.0` | string | IP de bind do servidor |
| `PORT` | `8080` | u16 | Porta HTTP |
| `MAX_JSON_PAYLOAD_SIZE` | `52428800` | bytes | Limite payload JSON (50MB) |
| `MAX_RAW_PAYLOAD_SIZE` | `52428800` | bytes | Limite payload raw (50MB) |
| `FEATURE_CACHE_TTL` | `10` | segundos | TTL do cache de feature flags |

## Autenticação

| Variável | Default | Tipo | Descrição |
|---|---|---|---|
| `API_AUTH_ENABLED` | `false` | bool | Ativa/desativa Bearer Token auth |
| `API_TOKEN_EXPIRY_DAYS` | `∞` | u32 | Expiração do token em dias |

## Storage Engine

| Variável | Default | Tipo | Descrição | Impacto |
|---|---|---|---|---|
| `DIR_PATH` | `./data` | path | Diretório de dados (WAL + SST) | — |
| `MEMTABLE_MAX_SIZE` | `16777216` | bytes | Tamanho máximo da MemTable (16MB) | ↑ = menos flushes, mais RAM |
| `BLOCK_SIZE` | `4096` | bytes | Tamanho de bloco SSTable | ↑ = menos I/Os, mais compressão |
| `BLOCK_CACHE_SIZE_MB` | `64` | MB | Tamanho do Block Cache LRU | ↑ = mais hits, mais RAM |
| `BLOOM_FALSE_POSITIVE_RATE` | `0.01` | float | Taxa de falso positivo do Bloom Filter | ↓ = menos I/Os, mais RAM |
| `INDEX_INTERVAL` | `16` | número | Intervalo do Sparse Index (1 entrada a cada N blocos) | ↓ = busca mais precisa, mais RAM |

## Tuning por cenário

### Write-heavy (ingest de dados, benchmarks)
```env
MEMTABLE_MAX_SIZE=67108864   # 64MB — menos flushes
BLOCK_CACHE_SIZE_MB=32       # menos RAM para cache
BLOOM_FALSE_POSITIVE_RATE=0.05  # aceitar mais false positives
```

### Read-heavy (dashboard, consultas frequentes)
```env
MEMTABLE_MAX_SIZE=8388608    # 8MB — flushes mais rápidos
BLOCK_CACHE_SIZE_MB=256      # cache grande
BLOOM_FALSE_POSITIVE_RATE=0.001  # false positives mínimos
INDEX_INTERVAL=4             # index mais denso
```

### Desenvolvimento local
```env
MEMTABLE_MAX_SIZE=1048576    # 1MB — testa flush rapidamente
BLOCK_CACHE_SIZE_MB=8
API_AUTH_ENABLED=false
```

## Como o config é carregado

`src/infra/config.rs` usa `dotenvy` para ler `.env` e `std::env::var` para cada campo. Erros de parsing resultam em `ApexError::ConfigError` e encerram o processo no startup. Não há hot-reload de config — restart necessário.
