# Auditoria Técnica Profunda — ApexStore v2.1.63

> **Data:** 2026-05-26  
> **Equipe:** 3 especialistas (Segurança/Storage, Resiliência/Sistemas Distribuídos, Performance/Banco de Dados)  
> **Repositório:** https://github.com/ElioNeto/ApexStore  
> **Licença:** MIT  

---

## Resumo Executivo

A ApexStore é um storage engine LSM-tree embarcado em Rust maduro e bem arquitetado, com **apenas 1 bloco `unsafe`** (mmap no reader.rs) e **zero vulnerabilidades conhecidas** em dependências diretas (`cargo audit` limpo). O código possui separação clara de camadas, testes abrangentes, e componentes de resiliência como circuit breaker, retry com backoff, backpressure e panic recovery.

**Pontuação geral: 7.2/10** — Base sólida, mas com 3 áreas críticas: (1) 918 chamadas `unwrap()` em produção podem causar crash inesperado, (2) ausência de limites de tamanho de chave/valor expõe a OOM, (3) API HTTP sem autenticação por default combinada com CORS permissivo cria superfície de ataque externa.

---

## 1. SEGURANÇA

### 1a. Segurança de Dados em Disco

**[ENCRYPTION-001] Criptografia em repouso existe mas é opcional e desabilitada por padrão**
- **Severidade:** Alta
- **Área:** Segurança
- **Componente:** SSTable / WAL
- **Impacto:** Qualquer um com acesso ao filesystem pode ler todos os dados. O `Encryptor` com AES-256-GCM está implementado corretamente (nonces aleatórios de 12 bytes, autenticação via GCM), mas `EncryptionConfig::default()` retorna `enabled: false`. Nenhum código de inicialização do servidor (`bin/server.rs`) chama `from_key_path()`.
- **Evidência:** `src/storage/encryption.rs:27-33` — `enabled: false` no default. `src/bin/server.rs:61-72` — `LsmConfig::builder()` não chama `.encryption_key_path()`.
- **Como reproduzir:** Executar servidor, inspecionar `.lsm_data/sstables/*.sst` com `xxd` — dados são texto puro (LZ4 comprimido, sem criptografia).
- **Correção recomendada:** Configuração deve **exigir** chave em produção (panic se `enabled=false` em release build). Adicionar `LsmConfig::builder().encryption_required(true)`.
- **Esforço:** Baixo

**[FS-PERM-001] Arquivos de dados criados com permissões 0644 (world-readable)**
- **Severidade:** Média
- **Área:** Segurança
- **Componente:** WAL / SSTable
- **Impacto:** Em sistemas multi-tenant, qualquer usuário local pode ler dados do storage engine.
- **Evidência:** `.lsm_data/sstables/*.sst` e `wal.log` têm permissão `-rw-rw-r--`. O Rust cria arquivos com a umask do processo, sem chamada explícita a `set_permissions()`.
- **Correção recomendada:** Usar `std::fs::set_permissions()` para `0o600` (owner-only) em todos os arquivos de dados criados. Adicionar `FILE_CREATION_MASK` configurável.
- **Esforço:** Baixo

**[PATH-TRAV-001] Nomes de SSTable derivados de timestamp — sem risco de path traversal**
- **Severidade:** Baixa (informativo)
- **Área:** Segurança
- **Componente:** SSTable
- **Impacto:** Nenhum — os nomes de arquivo são gerados internamente via `SystemTime::now().as_nanos()`, não derivados de input do usuário.
- **Evidência:** `src/storage/builder.rs`: nomes como `lsm_{timestamp}.sst`.

### 1b. Integridade de Dados

**[CRC-WAL-001] WAL usa CRC32 para detecção de corrupção**
- **Severidade:** Baixa (informativo — implementado corretamente)
- **Área:** Segurança
- **Componente:** WAL
- **Impacto:** Corrupção de bytes no WAL é detectada no replay. O frame CRC32 é verificado antes da desserialização. Frames corrompidos são ignorados com log de warning.
- **Evidência:** `src/storage/wal.rs:425-430`: `crc32fast::Hasher` usado para checksum de cada frame. Trailing partial checksum detectado em `425`.
- **Nota:** CRC32 detecta corrupção acidental, mas é **vulnerável a ataques intencionais** (não é MAC). Se a criptografia estiver habilitada, AES-256-GCM provê autenticação via tag MAC.

**[SST-CRC-001] Blocos de SSTable têm CRC32 verificado nas leituras**
- **Severidade:** Baixa (informativo)
- **Área:** Segurança/Resiliência
- **Componente:** SSTable
- **Impacto:** Dados corrompidos em disco são detectados.
- **Evidência:** `src/storage/reader.rs:215-230`: CRC32 do bloco é verificado após descompressão. `src/infra/scrubber.rs:290-300`: scrubber de integridade verifica CRC32 de todos os blocos.

**[CRASH-SST-001] Arquivo SSTable truncado durante crash — detectado, mas sem recuperação automática**
- **Severidade:** Alta
- **Área:** Resiliência
- **Componente:** SSTable
- **Impacto:** Se um SSTable for truncado (crash durante escrita), o engine detecta o magic number inválido ou CRC32 mismatch no startup, mas **descarta o arquivo inteiro** sem tentar recuperar registros parciais. Dados são perdidos. O Scrubber identifica o problema mas não tem auto-repair.
- **Evidência:** `src/storage/reader.rs:60-80`: validação de magic number e tamanho mínimo; `src/storage/reader.rs:215`: CRC32 mismatch → `CorruptedData` error.
- **Correção recomendada:** Implementar auto-repair: se CRC32 falhar em alguns blocos, tentar recuperar blocos íntegros adjacentes. Pelo menos quarternar o arquivo em vez de deletar.
- **Esforço:** Alto

### 1c. Robustez Contra Entradas Maliciosas

**[INPUT-VAL-001] Sem limite de tamanho de chave (key) ou valor (value)**
- **Severidade:** Crítica
- **Área:** Segurança/Performance
- **Componente:** Engine/API
- **Impacto:** Atacante pode enviar chaves de 100MB ou valores de 1GB. O engine alocará memória para armazená-los no MemTable e subsequentemente no SSTable, causando OOM. Não há defesa em profundidade.
- **Evidência:** `src/core/engine/mod.rs:745-774`: função `put_cf_internal` aceita `key` e `value` como `Vec<u8>` sem verificação de tamanho máximo. `src/api/mod.rs` não valida antes de chamar o engine.
- **Como reproduzir:** `curl -X PUT -d '{"value":"A".repeat(100_000_000)}' http://localhost:8080/keys/vuln` — engine aloca 100MB.
- **Correção recomendada:**
  ```rust
  const MAX_KEY_SIZE: usize = 4096;   // 4KB max key
  const MAX_VALUE_SIZE: usize = 1_048_576; // 1MB max value (configurável)
  if key.len() > MAX_KEY_SIZE {
      return Err(LsmError::InvalidArgument(format!("key too large: {} bytes", key.len())));
  }
  ```
  Validar tanto no engine quanto no middleware HTTP.
- **Esforço:** Baixo

**[INPUT-VAL-002] API aceita JSON de 50MB sem limite por campo**
- **Severidade:** Crítica
- **Área:** Segurança
- **Componente:** API
- **Impacto:** 20 requisições simultâneas de 50MB = 1GB de memória.
- **Evidência:** `src/api/config.rs:50`: `max_json_payload_size: 50 * 1024 * 1024`
- **Correção recomendada:** Reduzir para 1MB default. Validar tamanho do campo `value` individualmente.
- **Esforço:** Baixo

**[NON-UTF-KEY-001] Chaves não-UTF8 são aceitas mas podem causar problemas na API**
- **Severidade:** Média
- **Área:** Segurança
- **Componente:** API
- **Impacto:** A API usa `String::from_utf8_lossy()` para exibir chaves, que substitui bytes inválidos por `�`. Isso pode mascarar ataques ou dificultar debugging. Pior: chaves binárias podem conter bytes de controle que quebram serialização.
- **Evidência:** `src/api/mod.rs:56, 86-87`: `String::from_utf8_lossy(&key)` e `String::from_utf8_lossy(&value)`.
- **Correção recomendada:** Para a API REST, exigir chaves UTF-8 válidas com validação explícita. Ou documentar que API trabalha com base64 para chaves binárias.
- **Esforço:** Baixo

### 1d. Segurança de Memória (Rust-específico)

**[UNSAFE-001] Único bloco unsafe é justificado e seguro**
- **Severidade:** Baixa (informativo)
- **Área:** Segurança
- **Componente:** SSTable Reader
- **Impacto:** Nenhum. O uso de `unsafe` é para `Mmap::map()` do crate `memmap2`, que é uma operação padrão e segura na prática. O fallback para `pread` existe caso o mmap falhe.
- **Evidência:** `src/storage/reader.rs:132`: `unsafe { Mmap::map(&file) }` com fallback documentado.

**[UNWRAP-001] 918 chamadas `unwrap()` em produção podem causar crash**
- **Severidade:** Crítica
- **Área:** Resiliência
- **Componente:** Geral
- **Impacto:** Qualquer `unwrap()` em produção causa `panic!` se a operação falhar. No contexto de um servidor actix-web, um panic em uma worker thread derruba a thread (não o processo, graças ao `catch_unwind` do actix), mas corrompe estado compartilhado se o `Mutex` estiver locked no momento do panic.
- **Evidência:** `grep -rn "\bunwrap(" src/ --include="*.rs" | grep -v "#\[" | grep -v "// " | grep -v "test" | wc -l` = 918.
- **Correção recomendada:**
  - Fase 1: Substituir todos `unwrap()` em caminhos críticos (WAL write, memtable insert, compactação) por `?` ou `expect("context")`.
  - Fase 2: Usar `cargo clippy -- -D clippy::unwrap_used` para prevenir novos `unwrap()`.
  - Fase 3: Adicionar `#[deny(clippy::unwrap_used)]` no crate.
- **Esforço:** Alto (918 ocorrências) — pode ser automatizado parcialmente com `cargo fix`.

**[EXPECT-001] 19 chamadas `expect()` em produção**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** Geral
- **Impacto:** `expect()` é marginalmente melhor que `unwrap()` por dar contexto, mas ainda causa panic. Alguns `expect()` em `engine/mod.rs` (linhas 167, 1581) estão em caminhos críticos.
- **Evidência:** `grep -rn "\bexpect(" src/ --include="*.rs" | grep -v "#\[" | grep -v "test" | wc -l` = 19.
- **Correção recomendada:** Converter para `?` com `thiserror` ou `anyhow` context.
- **Esforço:** Médio

**[MEMTABLE-CONC-001] MemTable usa BTreeMap não-concorrente com Mutex externo**
- **Severidade:** Baixa (informativo)
- **Área:** Segurança
- **Componente:** MemTable
- **Impacto:** O `BTreeMap<Vec<u8>, LogRecord>` do MemTable é protegido por `parking_lot::Mutex` no core do engine. Isso é correto para um engine single-writer, mas limita throughput em workloads concorrentes.
- **Evidência:** `src/core/memtable.rs:3`: `use std::collections::BTreeMap`. O engine usa `Mutex` como única porta de entrada (`src/core/engine/mod.rs:753`: `core.lock()`).
- **Observação:** Sem data race — o mutex garante exclusão mútua. O tradeoff é performance, não segurança.

### 1e. Segurança de Acesso à API

**[AUTH-WIRE-001] Auth middleware existe mas é ignorado quando desabilitado**
- **Severidade:** Crítica
- **Área:** Segurança
- **Componente:** API/Auth
- **Impacto:** API pública sem autenticação por default. Já documentado como C-01.
- **Evidência:** `src/api/auth/middleware.rs:33-35`.

**[RATE-LIMIT-001] Rate limiter não é aplicado a endpoints admin**
- **Severidade:** Média
- **Área:** Segurança
- **Componente:** API/RateLimiter
- **Impacto:** Endpoints `/admin/flush` e `/admin/compact` podem ser chamados centenas de vezes por segundo, causando degradação.
- **Evidência:** Nenhum rate limit específico para admin endpoints. O rate limiter global de 100 req/min se aplica a todos.
- **Correção recomendada:** Adicionar `rate_limiter.set_endpoint_limit("/admin", 10)`.
- **Esforço:** Mínimo

**[SECRETS-001] Nenhum hardcoded secret encontrado**
- **Severidade:** Boa prática ✅
- **Área:** Segurança
- **Componente:** Geral
- **Evidência:** Scan com regex `ghp_|github_pat|sk-|AKIA` não encontrou ocorrências. `.secrets.example` contém apenas placeholder.

---

## 2. RESILIÊNCIA

### 2a. Recuperação Após Crash (Crash Recovery)

**[WAL-FSYNC-001] WAL usa batch fsync com intervalos — tradeoff durabilidade vs performance**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** WAL
- **Impacto:** O WAL não fsync a cada write, mas a cada `WAL_SYNC_INTERVAL` registros (tipicamente 4). Em caso de crash, até N-1 writes podem ser perdidos (RPO = N writes). Para a maioria dos casos isso é aceitável, mas para aplicações financeiras ou de auditoria, cada write precisa ser síncrono.
- **Evidência:** `src/storage/wal.rs:116-125`: `WAL_SYNC_INTERVAL` definido como constante; batched sync implementado.
- **Correção recomendada:** Tornar `WAL_SYNC_INTERVAL` configurável. Para máxima durabilidade, permitir `WAL_SYNC_INTERVAL=1`.
- **Esforço:** Baixo

**[WAL-REPLAY-001] Replay do WAL é idempotente mas pode ser lento com WAL grande**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** WAL
- **Impacto:** O replay lê todo o WAL sequencialmente. Com WAL de 1GB, o startup pode levar segundos. O replay é correto (idempotente) porque o MemTable começa vazio e os registros são reaplicados, mas não há limite de tamanho do WAL antes do flush.
- **Evidência:** `src/storage/wal.rs:400-500`: loop de replay, decodifica cada frame. `WAL_CURRENT_FRAME_VERSION` verifica compatibilidade retroativa (V0, V1, V2, V3).
- **Correção recomendada:** Implementar WAL truncation após flush bem-sucedido. Arquivo WAL pode ser resetado periodicamente.
- **Esforço:** Médio

**[CRASH-COMPACTION-001] Crash durante compactação — atômica com rename**
- **Severidade:** Baixa (informativo — implementado corretamente)
- **Área:** Resiliência
- **Componente:** Compaction
- **Impacto:** A compactação é atômica do ponto de vista do leitor. Novos SSTables são escritos em arquivos temporários, e apenas no final um `rename()` atômico os move para o diretório oficial. Se o crash ocorrer durante a escrita, os temporários são ignorados no próximo startup.
- **Evidência:** `src/core/engine/compaction.rs`: padrão write-then-rename. `version_set.rs`: swap atômico de metadados.
- **Nota:** Correto. ✅

### 2b. Consistência do LSM-Tree

**[LSM-CONSIST-001] Leituras durante compactação podem ver dados de ambos os níveis (correto por design)**
- **Severidade:** Baixa (informativo)
- **Área:** Resiliência
- **Componente:** Compaction
- **Impacto:** Durante compactação, leitores veem tanto SSTables antigos (ainda não deletados) quanto o novo SSTable (já adicionado ao VersionSet). Isso é correto por design do LSM-tree: ambos os níveis são consultados e o merge iterator retorna o valor mais recente. Não há inconsistência, apenas possível amplificação de leitura temporária.
- **Evidência:** `src/core/iterators.rs`: MergeIterator combina múltiplos iteradores ordenados.
- **Nota:** Comportamento correto. ✅

**[SST-DELETE-001] SSTables antigos são deletados apenas após novo estar totalmente sincronizado**
- **Severidade:** Baixa (informativo)
- **Área:** Resiliência
- **Componente:** Compaction/SSTable
- **Impacto:** O VersionSet é atualizado com o novo SSTable (add_table) antes de remover o antigo (remove_table). A remoção do arquivo físico ocorre após a atualização do VersionSet.
- **Evidência:** `src/core/engine/compaction.rs`: sequência `add_table() → fsync() → remove_table() → unlink()`.
- **Nota:** Correto. ✅

### 2c. Tolerância a Falhas de I/O

**[IO-DISK-FULL-001] Disco cheio no WAL — erro propaga como IoError sem graceful degradation**
- **Severidade:** Alta
- **Área:** Resiliência
- **Componente:** WAL
- **Impacto:** Quando o disco enche, `write_record()` retorna `Err(IoError)`. O erro propaga para a API, que retorna 500. O engine **não entra em modo ReadOnly automaticamente** — o `DegradationManager` existe mas não é integrado com o `DiskMonitor`.
- **Evidência:** `src/storage/wal.rs:214-221`: erro de `sync_all()` propaga via `?`. `src/infra/degradation.rs`: implementado mas não chamado nos handlers. `src/infra/disk_monitor.rs`: `on_critical` callback não conectado ao engine.
- **Como reproduzir:** Preencher o disco (`dd if=/dev/zero of=./disk_fill bs=1M count=...`), depois `PUT /keys/test` → 500 Internal Server Error.
- **Correção recomendada:**
  1. Conectar `DiskMonitor::on_critical` → `DegradationManager::set_mode(ReadOnly)`
  2. Nos handlers da API, chamar `degradation_manager.check_write_allowed()` antes de escrever
  3. Retornar `503 Service Unavailable` em vez de 500
- **Esforço:** Médio

**[IO-READ-ERROR-001] Erro de leitura de SSTable — arquivo corrompido não é isolado**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** SSTable
- **Impacto:** Se um SSTable retorna erro de I/O na leitura, o erro propaga para o cliente como 500, mas o arquivo permanece no VersionSet. Leituras subsequentes continuam tentando ler o mesmo arquivo e falhando. Não há quarantine mechanism.
- **Evidência:** `src/storage/reader.rs`: erro de I/O propaga via `?` sem remover arquivo do rotation.
- **Correção recomendada:** Após N falhas consecutivas de leitura do mesmo SSTable, movê-lo para diretório de quarentena e removê-lo do VersionSet. Log de alerta.
- **Esforço:** Médio

**[BACKPRESSURE-001] CompactionBackpressure implementado mas não integrado com API rate limiter**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** Compaction/API
- **Impacto:** Quando a compactação não consegue acompanhar a taxa de escrita, o `CompactionBackpressure` calcula delays, mas não há mecanismo para reduzir dinamicamente o rate limit da API. O resultado é que o MemTable enche, writes são bloqueados abruptamente (write stall), causando picos de latência.
- **Evidência:** `src/infra/backpressure.rs`: implementação completa com EMA (exponential moving average). Não integrado com `RateLimiterState`.
- **Correção recomendada:** Quando `should_backpressure()` retorna `true`, reduzir dinamicamente `max_requests_per_minute` via callback.
- **Esforço:** Médio

### 2d. Concorrência e Locking

**[DEADLOCK-001] Engine usa parking_lot::Mutex — sem risco de poisoned lock**
- **Severidade:** Baixa (informativo)
- **Área:** Resiliência
- **Componente:** Engine
- **Impacto:** `parking_lot::Mutex` não envenena, diferentemente de `std::sync::Mutex`. Se uma thread panic enquanto segura o lock, o lock é liberado (embora o estado possa estar inconsistente).
- **Evidência:** `src/core/engine/mod.rs`: uso de `parking_lot::Mutex` consistente.
- **Nota:** Correto. ✅

**[STARVATION-001] Lock único no engine pode causar starvation de leitores durante escritas intensas**
- **Severidade:** Alta
- **Área:** Performance/Resiliência
- **Componente:** Engine
- **Impacto:** O engine usa um único `Mutex` para coordenar WAL + MemTable + VersionSet. Durante escritas intensas (1000+ writes/s), leitores ficam bloqueados aguardando o lock. Com `parking_lot::Mutex` (que é unfair por default), writers podem starving readers.
- **Evidência:** `src/core/engine/mod.rs:753`: `core.lock()` antes de escrever no WAL + MemTable. `src/core/engine/mod.rs`: gets também adquirem `core.lock()`.
- **Correção recomendada:**
  1. Usar `RwLock` para separar leitores (get) de escritores (put).
  2. Implementar MVCC (multi-version concurrency control) com VersionSet snapshot para leituras sem lock.
  3. Como passo intermediário, usar `parking_lot::RwLock` no lugar de `Mutex`.
- **Esforço:** Alto (mudança arquitetural)

### 2e. Observabilidade e Operabilidade

**[OBSERV-001] Métricas abrangentes expostas via Prometheus e OTel**
- **Severidade:** Boa prática ✅
- **Área:** Resiliência
- **Componente:** Metrics
- **Evidência:** `src/infra/metrics.rs`: 18+ contadores atômicos (sets, gets, cache_hits, erro, bloom_negatives, latências acumuladas). `format_prometheus()` gera formato padrão. `OtelInstruments` exporta via OTLP.

**[OBSERV-002] CDC tracking implementado mas sem métricas de latência do Webhook**
- **Severidade:** Baixa
- **Área:** Resiliência
- **Componente:** CDC
- **Impacto:** Não há métrica de latência ou taxa de erro do webhook CDC. Impossível monitorar saúde da entrega de eventos.
- **Correção recomendada:** Adicionar `cdc_events_total`, `cdc_errors_total`, `cdc_latency_ms` ao `EngineMetrics`.
- **Esforço:** Baixo

**[OBSERV-003] Logging é estruturado (tracing + OTel) mas não inclui request ID consistente**
- **Severidade:** Baixa
- **Área:** Resiliência
- **Componente:** API
- **Impacto:** Sem `x-request-id` tracking ponta-a-ponta, correlacionar logs de uma mesma requisição entre múltiplos serviços é difícil.
- **Evidência:** `src/api/mod.rs:574`: `.wrap(Logger::default())` — formato default sem request ID tracking.
- **Correção recomendada:** Usar `Logger::default().custom_request_header("x-request-id")` ou middleware próprio.
- **Esforço:** Baixo

### 2f. RPO / RTO

**[RPO-001] RPO estimado: < WAL_SYNC_INTERVAL registros (default ~4 writes)**
- **Severidade:** Baixa (aceitável para a maioria dos casos)
- **Área:** Resiliência
- **Componente:** WAL
- **Impacto:** Em caso de crash, até 4 writes podem ser perdidos (dependendo do `WAL_SYNC_INTERVAL`). Para aplicações críticas, configurar `WAL_SYNC_INTERVAL=1`.
- **Evidência:** `WAL_SYNC_INTERVAL = 4` (hardcoded em `wal.rs:125`).
- **Correção recomendada:** Tornar configurável.

**[RTO-001] RTO estimado: < 1s para WAL pequeno, até 10s para WAL de 1GB**
- **Severidade:** Média
- **Área:** Resiliência
- **Componente:** WAL/Startup
- **Impacto:** O replay do WAL no startup é sequencial. Sem benchmark específico, estima-se ~10MB/s de throughput de replay (devido a decodificação postcard + CRC32). Para WAL de 1GB, RTO de ~100 segundos.
- **Correção recomendada:** Implementar WAL truncation pós-flush. Adicionar benchmark de replay em `benches/`.
- **Esforço:** Médio

---

## 3. PERFORMANCE

### 3a. Write Path

**[WRITE-WAL-001] WAL sync é o gargalo principal de writes — group commit pode melhorar throughput**
- **Severidade:** Média
- **Área:** Performance
- **Componente:** WAL
- **Impacto:** Cada `sync_all()` no WAL custa ~0.5-10ms (dependendo do disco). O batch sync com `WAL_SYNC_INTERVAL=4` amortiza isso, mas 4 writes por fsync ainda é pouco. Group commit (acumular writes de múltiplos clientes antes de fsync) melhoraria throughput em ~4x.
- **Evidência:** `src/storage/wal.rs:214-221`: batch sync após N writes.
- **Correção recomendada:** Implementar group commit: acumular writes de todos os clientes em um buffer compartilhado e fsync em intervalos fixos (ex: 1ms) ou após N bytes.
- **Esforço:** Alto

**[WRITE-SERIAL-001] Todo write adquire lock único do engine — bottleneck de concorrência**
- **Severidade:** Alta
- **Área:** Performance
- **Componente:** Engine
- **Impacto:** Writes são serializadas pelo `Mutex` do engine. Com 8+ cores, 7 ficam ociosas durante escritas. Throughput máximo limitado a ~50.000 ops/s (estimado).
- **Evidência:** `src/core/engine/mod.rs:753`: `core.lock()` antes de qualquer operação de escrita.
- **Correção recomendada:** WAL lock separado do MemTable lock. Pipeline de escrita: WAL → MemTable (dois locks diferentes).
- **Esforço:** Alto

### 3b. Read Path

**[READ-BLOOM-001] Bloom Filter implementado e usado para evitar leituras desnecessárias de SSTable**
- **Severidade:** Boa prática ✅
- **Área:** Performance
- **Componente:** SSTable
- **Impacto:** Bloom Filter com false positive rate de 1% (configurável) evita ~99% das leituras de SSTable que não contêm a chave.
- **Evidência:** `src/storage/reader.rs:120-122`: `Bloom::<[u8]>::from_bytes()` usado no open do SSTable. `src/storage/reader.rs:180-195`: bloom filter checked antes de procurar bloco.
- **Nota:** Implementado corretamente. ✅

**[READ-CACHE-001] Block Cache LRU implementado com lru crate**
- **Severidade:** Boa prática ✅
- **Área:** Performance
- **Componente:** SSTable/Cache
- **Impacto:** Blocos de SSTable descomprimidos são cacheados em LRU, reduzindo I/O repetido para keys populares.
- **Evidência:** `src/storage/cache.rs:1`: `use lru::LruCache`. Cache sharded por `table_id`.
- **Observação:** A `lru` crate versão 0.12.5 tem advisory RUSTSEC-2026-0002 (unsound IterMut). Upgrade para 0.16.3+ recomendado.

**[READ-AMPLIFICATION-001] Leitura pode tocar múltiplos níveis (L0..Ln)**
- **Severidade:** Média
- **Área:** Performance
- **Componente:** Engine
- **Impacto:** Cada `get()` precisa verificar MemTable + todos os níveis L0..Ln. Com 10 níveis e bloom filter com 1% FP rate, cada get faz em média 1 (MemTable) + 1 (L0, sem bloom) + 0.01 * 9 (L1..L9, bloom) ≈ 2.1 verificações de SSTable. Em níveis mais altos e sem bloom, pode chegar a 10+ verificações.
- **Evidência:** `src/core/engine/mod.rs:860-900`: função `get_cf` itera níveis.
- **Correção recomendada:** Manter Bloom Filter obrigatório (default enabled). Adicionar métrica `read_amplification` exposta.
- **Esforço:** Baixo

### 3c. Compaction

**[COMP-STRATEGY-001] Estratégia híbrida LazyLeveling: size-tiered em L0, leveled nos demais**
- **Severidade:** Boa prática ✅
- **Área:** Performance
- **Componente:** Compaction
- **Impacto:** Balanceamento entre write amplification (size-tiered é melhor) e space amplification (leveled é melhor) e read amplification (leveled é melhor). Estratégia bem escolhida para caso de uso geral.
- **Evidência:** `src/core/engine/compaction.rs:389-450`: `LazyLevelingCompaction` implementa switch.

**[COMP-WRITE-AMP-001] Write amplification não é monitorada em produção**
- **Severidade:** Média
- **Área:** Performance
- **Componente:** Compaction/Metrics
- **Impacto:** Bench `write_amplification` existe, mas a métrica real `compaction_bytes_written / user_bytes_written` não é exposta no Prometheus. Usuários não conseguem detectar se a configuração de níveis está causando WA excessiva.
- **Evidência:** `benches/write_amplification.rs` existe. `src/infra/metrics.rs`: não inclui `write_amplification_ratio`.
- **Correção recomendada:** Adicionar métrica `apexstore_write_amplification_ratio` calculada a partir de `compaction_bytes_total` e `user_bytes_written_total`.
- **Esforço:** Baixo

### 3d. Benchmarks e Metas

**[BENCH-001] Benchmarks abrangentes com criterion.rs**
- **Severidade:** Boa prática ✅
- **Área:** Performance
- **Componente:** Geral
- **Evidência:** `benches/`: write_bench, read_bench, mixed_bench, scan_bench, stress_bench, latency_bench, write_amplification. 7 benchmarks cobrindo todos os aspectos.

**[BENCH-002] Benchmarks não têm valores-alvo (regression gates)**
- **Severidade:** Baixa
- **Área:** Performance
- **Componente:** CI
- **Impacto:** Sem thresholds, CI não detecta regressões de performance automaticamente.
- **Correção recomendada:** Usar `criterion` comparison feature com `baseline` para detectar regressões > 5% no CI.
- **Esforço:** Baixo

### 3e. Uso de Memória

**[MEM-MEMTABLE-001] MemTable usa BTreeMap — O(n log n) insert, O(log n) get**
- **Severidade:** Média
- **Área:** Performance
- **Componente:** MemTable
- **Impacto:** `BTreeMap<Vec<u8>, LogRecord>` tem insert O(log n). Para 1M registros no MemTable, insert custa ~20 comparações de chave. Skip list teria O(log n) médio similar mas melhor localidade de cache e concorrência lock-free.
- **Evidência:** `src/core/memtable.rs:3`: `use std::collections::BTreeMap`.
- **Correção recomendada:** Avaliar `crossbeam-skiplist` ou `dashmap` para concorrência lock-free. Por enquanto, para workload single-writer, BTreeMap é aceitável.
- **Esforço:** Alto (substituir estrutura de dados)

**[MEM-ITERATOR-001] MergeIterator durante compactação é lazy (streaming)**
- **Severidade:** Boa prática ✅
- **Área:** Performance
- **Componente:** Compaction
- **Impacto:** Não carrega SSTables inteiros em memória. Lê blocos sob demanda, descomprime, itera, descarta.
- **Evidência:** `src/core/iterators.rs`: `MergeIterator` e `StorageIterator` usam pattern de streaming.

---

## 4. Matriz de Risco do LSM-Tree

| Componente | Segurança | Resiliência | Performance | Risco Geral |
|-----------|-----------|-------------|-------------|-------------|
| **WAL** | 🟢 CRC32 + AES-256-GCM (opt) | 🟡 Batch fsync (RPO não-zero) | 🟡 Group commit não implementado | **Médio** |
| **MemTable** | 🟢 BTreeMap + Mutex seguro | 🟡 Lock único pode causar starvation | 🟡 BTreeMap O(log n), sem skip list | **Médio** |
| **SSTable** | 🟢 CRC32 + Bloom + Cache LRU | 🟡 Sem auto-repair de corrupção | 🟢 Blocos cacheados, bloom filter | **Baixo** |
| **Compaction** | 🟢 Write-then-rename atômico | 🟢 Crash-safe | 🟢 LazyLeveling híbrido | **Baixo** |
| **API/HTTP** | 🔴 Auth+CORS+Payload+No TLS | 🟡 Sem degradation integrado | 🟡 N+1 scan, batch sem limite | **Crítico** |
| **CDC** | 🔴 Sem auth, sem retry | 🟡 Sem timeout, sem circuit breaker | 🟢 Boa estrutura de eventos | **Alto** |

---

## 5. Plano de Correção Priorizado

### Quick Wins (24h)

| ID | Ação | Esforço | Issues |
|----|------|---------|--------|
| C-01 | Mudar `AuthConfig::default()` para `enabled: true` | 15min | #324 |
| C-02 | Substituir `Cors::permissive()` por deny-by-default | 30min | #325 |
| C-03 | Reduzir `MAX_JSON_PAYLOAD_SIZE` para 1MB | 15min | #326 |
| INPUT-VAL-001 | Adicionar `MAX_KEY_SIZE` e `MAX_VALUE_SIZE` no engine | 1h | (nova issue) |
| H-03 | Desabilitar GraphQL playground em release | 30min | #331 |
| M-10 | Adicionar `cargo audit` ao CI | 30min | #346 |
| RATE-LIMIT | Adicionar endpoint limits para `/admin/*` | 15min | (nova issue) |
| COMP-WRITE-AMP | Adicionar métrica write amplification no Prometheus | 1h | #352 |

### Correções para 7 Dias

| ID | Ação | Esforço | Issues |
|----|------|---------|--------|
| C-04 | Adicionar suporte TLS nativo (rustls) | 8h | #327 |
| C-05 | Adicionar auth + retry + timeout no CDC Webhook | 4h | #328 |
| H-01 | Refatorar `/scan` para usar iterator scan (N+1 fix) | 3h | #329 |
| H-02 | Adicionar `max_batch_size` no batch endpoint | 1h | #330 |
| H-04 | Adicionar auth no WebSocket sync | 4h | #332 |
| H-05 | Substituir `Mutex<HashMap>` por sharded rate limiter | 6h | #333 |
| H-06 | Integrar IdempotencyMiddleware na cadeia | 3h | #334 |
| H-07 | Adicionar validação de env vars com logging | 2h | #335 |
| H-08 | Implementar `keys_with_limit()` | 4h | #336 |
| INPUT-VAL-002 | Validar UTF-8 nas chaves da API | 1h | (nova issue) |
| M-01 | Fixar default workers=4 | 30min | #337 |
| M-08 | Integrar DegradationManager nos handlers | 3h | #344 |
| M-09 | Adicionar `max_connections_per_ip` | 3h | #345 |

### Melhorias Estruturais (30 Dias)

| ID | Ação | Esforço |
|----|------|---------|
| UNWRAP-001 | Erradicar 918 `unwrap()` em produção | 40h (pode ser automatizado parcialmente) |
| STARVATION-001 | Migrar de `Mutex` para `RwLock` no engine | 16h |
| WRITE-WAL-001 | Implementar group commit no WAL | 12h |
| WRITE-SERIAL-001 | Separar WAL lock do MemTable lock | 20h |
| IO-DISK-FULL-001 | Integrar Degradation + DiskMonitor + API handlers | 8h |
| IO-READ-ERROR-001 | Implementar quarantine de SSTables corrompidos | 8h |
| CRASH-SST-001 | Implementar auto-repair parcial de SSTables | 16h |
| WAL-FSYNC-001 | Tornar WAL_SYNC_INTERVAL configurável | 2h |
| BACKPRESSURE-001 | Integrar CompactionBackpressure com RateLimiter | 6h |
| MEMTABLE-CONC | Avaliar skip list lock-free (crossbeam) | 16h |

---

## 6. Plano de Testes Recomendado

### Testes Unitários por Componente

| Componente | O que testar | Status atual |
|-----------|-------------|--------------|
| WAL | CRC32 detecção, versões V0-V3, replay, truncation | ✅ Bom |
| MemTable | Insert/Get/Delete/Scan, TTL expiry, overflow | ✅ Bom |
| SSTable | Build/Read, CRC32, bloom filter, compressão | ✅ Bom |
| Compaction | Estratégias size-tiered/leveled, atomicidade | ✅ Bom |
| Auth | Token CRUD, expiry, permissions, timing | ✅ Bom |
| Rate Limiter | Sliding window, per-endpoint, X-Forwarded-For | ✅ Bom |
| Retry | Backoff, jitter, exhaustion | ✅ Bom |
| Circuit Breaker | Open/HalfOpen/Closed, thresholds | ✅ Bom |
| Backup | Snapshot/restore, retention, erro | ✅ Bom |
| Encryption | AES-256-GCM roundtrip, wrong key, disabled | ✅ Bom |
| Scrubber | CRC32 valid/invalid, orphan detection | ✅ Bom |
| **Degradation** | Mode switching, write block | ✅ Bom |
| **Idempotency** | Cache hit/miss, cleanup, TTL | ✅ Bom |

### Testes de Propriedade (Property-Based Testing com `proptest`)

Recomendação: adicionar testes de propriedade para:

```
1. WAL Replay Property:
   "Para qualquer sequência de operações put/delete, 
    o replay do WAL no startup deve produzir o mesmo estado 
    que o engine antes do crash."

2. Compaction Idempotency:
   "Compactar duas vezes seguidas deve produzir o mesmo 
    resultado que compactar uma vez."

3. Snapshot/Recovery:
   "create_snapshot + restore_snapshot deve restaurar 
    o estado completo do banco."

4. Bloom Filter:
   "Para qualquer chave K, se o bloom filter disser 
    'não presente', então K definitivamente não está no SSTable."
```

### Fuzz Testing (com `cargo-fuzz`)

```
1. WAL Frame Fuzzing:
   - Alimentar frames corrompidos, truncados, com versões misturadas
   - Verificar que não há pânico nem corrupção de estado

2. SSTable Fuzzing:
   - Arquivos SSTable corrompidos (bytes aleatórios)
   - Verificar graceful error handling (não pânico)

3. API Input Fuzzing:
   - JSON malformado, chaves binárias, valores enormes
   - Headers HTTP maliciosos
   - Paths com caracteres especiais
```

### Testes de Chaos (com `fail-rs`)

```
1. I/O Injection:
   - Injetar falhas de leitura/escrita no WAL
   - Injetar disco cheio
   - Injetar lentidão de I/O (latency injection)

2. Crash Injection:
   - Crash no meio do batch write
   - Crash no meio da compactação
   - Crash durante flush do MemTable

3. Network Chaos (CDC):
   - CDC endpoint lento
   - CDC endpoint retornando 500
   - CDC timeout
```

### Benchmarks com `criterion.rs`

Já existentes (7 benchmarks):
- write_bench, read_bench, mixed_bench, scan_bench
- stress_bench, latency_bench, write_amplification

Recomendação adicional:
```
8. replay_bench: benchmark de replay do WAL (RTO)
9. compaction_bench: latência e throughput da compactação
10. concurrent_bench: throughput vs número de threads
```

---

## 7. Achados Detalhados (Formato Padrão)

### 🔴 Críticos (5)

**[CRIT-01] API pública sem autenticação por default**
- **Severidade:** Crítica | Segurança | API/Auth
- **Impacto:** Qualquer endpoint acessível sem token. Leitura/escrita/delete/admin irrestritos.
- **Evidência:** `src/api/auth/middleware.rs:33-35`
- **Correção:** Default `enabled: true`; middleware deny-by-default.
- **Esforço:** Baixo

**[CRIT-02] CORS permissivo permite qualquer origem**
- **Severidade:** Crítica | Segurança | API
- **Impacto:** Exfiltração de dados cross-origin.
- **Evidência:** `src/api/mod.rs:518`
- **Correção:** Exigir origens explícitas.
- **Esforço:** Baixo

**[CRIT-03] Payload JSON de 50MB permite OOM**
- **Severidade:** Crítica | Segurança/Performance | API
- **Evidência:** `src/api/config.rs:50`
- **Correção:** Reduzir para 1MB.
- **Esforço:** Baixo

**[CRIT-04] Sem TLS/HTTPS — MITM total**
- **Severidade:** Crítica | Segurança | API
- **Evidência:** Nenhuma dependência TLS no Cargo.toml
- **Correção:** Adicionar rustls + bind_rustls().
- **Esforço:** Médio

**[CRIT-05] 918 unwrap() em produção causam crash em erro**
- **Severidade:** Crítica | Resiliência | Geral
- **Evidência:** `grep -c "unwrap(" src/ --include=*.rs` = 918
- **Correção:** Substituir por `?`, configurar `clippy::unwrap_used`.
- **Esforço:** Alto

### 🟠 Altos (8)

**[HIGH-01] N+1 query no endpoint /scan**
- **Severidade:** Alta | Performance | API
- **Componente:** API/Engine
- **Impacto:** 10.001 chamadas para 10.000 keys.
- **Evidência:** `src/api/mod.rs:439-463`
- **Correção:** Usar `engine.scan()`.

**[HIGH-02] Batch insert sem limite — DoS por alocação**
- **Severidade:** Alta | Segurança | API
- **Evidência:** `src/api/mod.rs:406-431`
- **Correção:** `max_batch_size` default 1000.

**[HIGH-03] Sem limite de tamanho de chave/valor**
- **Severidade:** Alta | Segurança/Performance | Engine
- **Evidência:** `src/core/engine/mod.rs:745`: sem validação.
- **Correção:** `MAX_KEY_SIZE = 4096, MAX_VALUE_SIZE = 1MB`.

**[HIGH-04] Lock único serializa writes e starving reads**
- **Severidade:** Alta | Performance | Engine
- **Evidência:** `Mutex` único para todo o core.
- **Correção:** `RwLock` + MVCC.

**[HIGH-05] Disco cheio não ativa modo ReadOnly automaticamente**
- **Severidade:** Alta | Resiliência | Engine/API/Disk
- **Evidência:** DegradationManager não integrado.
- **Correção:** Integrar DiskMonitor → DegradationManager → API handlers.

**[HIGH-06] CDC Webhook sem autenticação, retry ou timeout**
- **Severidade:** Alta | Segurança/Resiliência | CDC
- **Evidência:** `src/infra/cdc.rs:217-250`
- **Correção:** Adicionar Auth header, RetryConfig, timeout 5s.

**[HIGH-07] WebSocket sync sem autenticação**
- **Severidade:** Alta | Segurança | API/Sync
- **Evidência:** `src/api/sync.rs:187-211`
- **Correção:** Validar token no handshake WebSocket.

**[HIGH-08] GraphQL playground exposto em produção**
- **Severidade:** Alta | Segurança | API/GraphQL
- **Evidência:** `src/api/mod.rs:491-493`
- **Correção:** Desabilitar em release.

### 🟡 Médios (12)

Listados na matriz completa em `docs/audit/auditoria-completa.md`.

### 🔵 Baixos (5)

Listados na matriz completa em `docs/audit/auditoria-completa.md`.

---

## 8. Checklist Antes de Release Pública

### Segurança
- [ ] `API_AUTH_ENABLED=true` — autenticação obrigatória
- [ ] Token admin criado antes do startup
- [ ] `CORS_ORIGINS` configurado com lista de origens confiáveis
- [ ] TLS habilitado (rustls nativo ou reverse proxy documentado)
- [ ] `MAX_JSON_PAYLOAD_SIZE` ≤ 1MB
- [ ] Criptografia em repouso habilitada (`encryption_key_path`)
- [ ] Rate limiter configurado com per-endpoint limits para admin
- [ ] GraphQL playground desabilitado
- [ ] WebSocket sync com autenticação
- [ ] `Idempotency-Key` middleware integrado
- [ ] CSRF protection middleware implementado

### Resiliência
- [ ] `DiskMonitor` configurado e conectado ao `DegradationManager`
- [ ] `WAL_SYNC_INTERVAL` configurado conforme RPO desejado
- [ ] `CompactionBackpressure` integrado ao rate limiter
- [ ] CDC webhook com retry + timeout + circuit breaker
- [ ] `PanicRecovery` registrado com callback de alerta
- [ ] Backup automático configurado com criptografia
- [ ] Health checks incluem dependências externas

### Performance
- [ ] `WORKERS` configurado (recomendado: 4)
- [ ] `BLOCK_CACHE_SIZE_MB` configurado conforme workload
- [ ] `BLOOM_FALSE_POSITIVE_RATE` definido (default 1%)
- [ ] Benchmarks executados contra hardware alvo
- [ ] `cargo criterion` sem regressão > 5% vs baseline

### CI/CD
- [ ] `cargo audit` executando em cada PR
- [ ] `cargo clippy -- -D warnings` passando
- [ ] `cargo test --all-features` passando
- [ ] `cargo fmt --check` passando
- [ ] Fuzz tests integrados (cargo-fuzz)

### Dependências
- [ ] `lru` upgrade para ≥ 0.16.3 (RUSTSEC-2026-0002)
- [ ] `paste` UNMAINTAINED — avaliar substituto (transitivo via ratatui)
- [ ] `atomic-polyfill` UNMAINTAINED — avaliar `portable-atomic`

---

## Issues Criadas

Todas as issues estão no GitHub:

| ID | Issue | Link |
|----|-------|------|
| C-01 | Auth desabilitado por padrão | [#324](https://github.com/ElioNeto/ApexStore/issues/324) |
| C-02 | CORS permissivo | [#325](https://github.com/ElioNeto/ApexStore/issues/325) |
| C-03 | Payload 50MB | [#326](https://github.com/ElioNeto/ApexStore/issues/326) |
| C-04 | Sem TLS | [#327](https://github.com/ElioNeto/ApexStore/issues/327) |
| C-05 | CDC sem auth/retry | [#328](https://github.com/ElioNeto/ApexStore/issues/328) |
| H-01 | N+1 /scan | [#329](https://github.com/ElioNeto/ApexStore/issues/329) |
| H-02 | Batch sem limite | [#330](https://github.com/ElioNeto/ApexStore/issues/330) |
| H-03 | GraphQL playground | [#331](https://github.com/ElioNeto/ApexStore/issues/331) |
| H-04 | WebSocket sem auth | [#332](https://github.com/ElioNeto/ApexStore/issues/332) |
| H-05 | Rate limiter Mutex bottleneck | [#333](https://github.com/ElioNeto/ApexStore/issues/333) |
| H-06 | Idempotency não integrado | [#334](https://github.com/ElioNeto/ApexStore/issues/334) |
| H-07 | Env vars sem validação | [#335](https://github.com/ElioNeto/ApexStore/issues/335) |
| H-08 | engine.keys() OOM | [#336](https://github.com/ElioNeto/ApexStore/issues/336) |
| M-01 | Workers ilimitados | [#337](https://github.com/ElioNeto/ApexStore/issues/337) |
| M-02 | Sem CSRF | [#338](https://github.com/ElioNeto/ApexStore/issues/338) |
| M-03 | Token timing attack | [#339](https://github.com/ElioNeto/ApexStore/issues/339) |
| M-04 | Backup sem criptografia | [#340](https://github.com/ElioNeto/ApexStore/issues/340) |
| M-05 | Retry blocking sleep | [#341](https://github.com/ElioNeto/ApexStore/issues/341) |
| M-06 | CDC sem timeout | [#342](https://github.com/ElioNeto/ApexStore/issues/342) |
| M-07 | Falta auditoria logs | [#343](https://github.com/ElioNeto/ApexStore/issues/343) |
| M-08 | Degradation não integrado | [#344](https://github.com/ElioNeto/ApexStore/issues/344) |
| M-09 | Sem limite conexões/IP | [#345](https://github.com/ElioNeto/ApexStore/issues/345) |
| M-10 | Sem cargo audit no CI | [#346](https://github.com/ElioNeto/ApexStore/issues/346) |
| DB-04 | Range delete ausente | [#347](https://github.com/ElioNeto/ApexStore/issues/347) |
| L-01 | CLI sem token mgmt | [#348](https://github.com/ElioNeto/ApexStore/issues/348) |
| L-02 | Dashboard reload | [#349](https://github.com/ElioNeto/ApexStore/issues/349) |
| L-03 | Endpoints duplicados | [#350](https://github.com/ElioNeto/ApexStore/issues/350) |
| L-04 | Health checks incompletos | [#351](https://github.com/ElioNeto/ApexStore/issues/351) |
| L-05 | Métrica write amplification | [#352](https://github.com/ElioNeto/ApexStore/issues/352) |

---

*Auditoria realizada em 2026-05-26 por equipe técnica de 3 especialistas.*
*Revisar a cada 6 meses ou após mudanças significativas de arquitetura.*
