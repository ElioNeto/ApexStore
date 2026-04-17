# Decisões de Arquitetura — ApexStore

Registro de decisões técnicas importantes (ADR simplificado).
Consulte antes de propor mudanças estruturais.

---

## ADR-001: bincode como formato de serialização em disco

**Status**: Aceito
**Data**: 2024

**Contexto**: Precisamos serializar `LogRecord` e blocos SSTable para disco com máxima eficiência.

**Decisão**: Usar `bincode` (formato binário compacto) para WAL e SSTables.

**Motivo**: ~3-5x menor que JSON, sem overhead de parsing textual, zero alocações desnecessárias. JSON é usado apenas na camada HTTP da API.

**Consequência**: Arquivos `.wal` e `.sst` não são human-readable. Inspecionar requer o `src/infra/codec.rs`.

---

## ADR-002: parking_lot em vez de std::sync

**Status**: Aceito
**Data**: 2024

**Contexto**: O Engine precisa de `RwLock` para múltiplos leitores concorrentes.

**Decisão**: Usar `parking_lot::RwLock` e `parking_lot::Mutex` em todo o código.

**Motivo**: `parking_lot` é não-reentrante por design (evita deadlocks acidentais), tem menor overhead de memória e não emite poison errors. API mais ergonômica (sem `.unwrap()` no lock).

**Consequência**: Nunca misturar com `std::sync`. Se um lock for adquirido, não chamar código que tente adquirir o mesmo lock (deadlock instantâneo, sem poison recovery).

---

## ADR-003: SSTable V2 com blocos + LZ4 + Sparse Index

**Status**: Aceito
**Data**: 2024

**Contexto**: SSTable V1 era um arquivo plano sem compressão nem index. Lento para arquivos grandes.

**Decisão**: SSTable V2 organiza dados em blocos de tamanho fixo (`BLOCK_SIZE=4096`), comprimidos individualmente com LZ4, com Sparse Index e Bloom Filter.

**Motivo**: LZ4 oferece compressão razoável (~2-3x) com decodificação extremamente rápida (~4GB/s). Sparse Index reduz RAM preservando localidade. Bloom Filter elimina 99% dos disk I/Os para chaves inexistentes.

**Consequência**: Mudança de formato é breaking. V1 não é compatível com V2. Migration guide em `MIGRATION_GUIDE.md`.

---

## ADR-004: Actix-Web como framework HTTP

**Status**: Aceito
**Data**: 2024

**Contexto**: Precisamos de um servidor HTTP de alta performance para expor o Engine como API REST.

**Decisão**: Usar Actix-Web 4 com Tokio.

**Motivo**: Actix-Web é consistentemente o framework Rust mais rápido em benchmarks (TechEmpower). Ecossistema maduro com `actix-cors`, `actix-web-httpauth` sem dependências extras.

**Consequência**: O Engine (síncrono com `parking_lot`) é exposto através de handlers async. Nunca chamar operações bloqueantes longas dentro de um handler sem `spawn_blocking`.

---

## ADR-005: Angular 17 standalone com Signals

**Status**: Aceito
**Data**: 2026

**Contexto**: Precisamos de um frontend para o dashboard da API.

**Decisão**: Angular 17 com componentes standalone, Signals para estado, nova template syntax.

**Motivo**: Signals eliminam Zone.js overhead. Standalone elimina boilerplate de NgModules. `@if`/`@for` são mais performantes que diretivas estruturais. Alinhado com o futuro do Angular (Signals-first).

**Consequência**: Não usar `ChangeDetectionStrategy.OnPush` com Signals (redundante). Não usar `async pipe` para Signals (usar `signal()` diretamente no template).

---

## ADR-006: Trunk-based development + auto-release

**Status**: Aceito
**Data**: 2024

**Contexto**: Queremos releases frequentes com mínimo de overhead manual.

**Decisão**: Toda feature vai para `main` via PR. O CI auto-incrementa `patch` no `Cargo.toml`, cria tag e GitHub Release.

**Motivo**: Elimina o problema de "quando fazer release". Todo merge é potencialmente releasável.

**Consequência**: PRs devem ser pequenos e sempre em estado releasável. Features grandes devem usar feature flags (`src/features/`) para não bloquear o trunk.
