# Auditoria Completa de Segurança, Resiliência e Performance — ApexStore

**Data:** 2026-05-26  
**Versão auditada:** 2.1.63  
**Escopo:** Código-fonte completo (Rust), configurações, dependências, infraestrutura  
**Metodologia:** Análise estática de código + revisão arquitetural + boas práticas OWASP/Cloud Native  

---

## Resumo Executivo

A ApexStore é um storage engine LSM-tree embarcado em Rust com maturidade arquitetural impressionante. O código é bem estruturado, com separação clara entre camadas, uso extensivo de `tracing`, `thiserror`, testes unitários e de integração, e componentes de resiliência como circuit breaker, retry com backoff, backpressure, disk monitor, watchdog e panic recovery.

**Pontuação geral: 6.5/10** — Código base robusto, mas com vulnerabilidades críticas de segurança na superfície HTTP.

### Principais descobertas

| Severidade | Contagem | Destaques |
|-----------|----------|-----------|
| 🔴 Crítico | 5 | Auth desabilitado por padrão, CORS permissivo, payload 50MB, sem TLS, injeção via CDC |
| 🟠 Alto | 8 | N+1 no scan, batch sem limite, GraphQL playground público, WebSocket sem auth, idempotência não integrada |
| 🟡 Médio | 10 | Workers ilimitados, sem CSRF, rate limiter com Mutex simples, token timing attack parcial, backup sem criptografia |
| 🔵 Baixo | 5 | CLI sem gestão de tokens, dashboard com auto-refresh, sem auditoria em CI |

---

## 🔴 Achados Críticos

### C-01: Autenticação Desabilitada por Padrão

| Campo | Valor |
|-------|-------|
| **Problema** | `API_AUTH_ENABLED=false` no `.env.example` e no `AuthConfig::default()`. Qualquer requisição sem token passa pelo `bearer_validator` sem verificação. |
| **Impacto** | Todos os endpoints (incluindo `/admin/flush`, `/admin/compact`, `/keys`, `/stats`) são **publicamente acessíveis** sem qualquer autenticação. Atacante pode ler, escrever, deletar qualquer dado e forçar flush/compaction. |
| **Evidência** | `src/api/config.rs:70-73`: `enabled: false`; `src/api/auth/middleware.rs:33-35`: `if !auth_enabled { return Ok(req); }` |
| **Recomendação** | Mudar default para `true` e exigir configuração explícita de `API_AUTH_ENABLED=false` em ambientes dev. Implementar fallback seguro (deny-by-default). |
| **Validação** | Teste de integração: tentar `GET /keys` sem `Authorization` header → deve retornar `401`. |
| **Prioridade** | Imediata |

### C-02: CORS Permissivo (Allow All Origins)

| Campo | Valor |
|-------|-------|
| **Problema** | `Cors::permissive()` é usado quando `cors_origins` é `None` (default). Permite qualquer origem, qualquer header, qualquer método com credentials. |
| **Impacto** | Qualquer site pode fazer requisições cross-origin autenticadas (se auth estivesse habilitado). Exfiltração de dados via cookies/session tokens. |
| **Evidência** | `src/api/mod.rs:518`: `None => actix_cors::Cors::permissive()`; `src/api/config.rs:61`: `cors_origins: None` |
| **Recomendação** | Nunca usar `.permissive()` em produção. Exigir lista explícita de origens ou usar validação dinâmica segura. |
| **Validação** | Teste: `curl -H "Origin: https://evil.com" -H "Authorization: Bearer ..." http://localhost:8080/keys` → verificar se `Access-Control-Allow-Origin` não reflete `evil.com`. |
| **Prioridade** | Imediata |

### C-03: Payload Máximo de 50MB

| Campo | Valor |
|-------|-------|
| **Problema** | `MAX_JSON_PAYLOAD_SIZE=52428800` (50MB) — extremamente alto para uma API key-value. Ataque DoS trivial com poucas conexões. |
| **Impacto** | 20 requisições simultâneas de 50MB = 1GB de memória. Engine LSM-tree não tem limite de tamanho de valor por chave. Pode causar OOM killer. |
| **Evidência** | `src/api/config.rs:50-51`: `max_json_payload_size: 50 * 1024 * 1024` |
| **Recomendação** | Reduzir para 1MB (1048576) como default. Implementar limite configurável por endpoint. Adicionar validação no middleware ANTES de chegar ao handler. |
| **Validação** | `curl -X PUT -d "$(python -c 'print("x"*2000000)')" http://localhost:8080/keys/test` → deve retornar 413 Payload Too Large. |
| **Prioridade** | Imediata |

### C-04: Ausência de TLS/HTTPS

| Campo | Valor |
|-------|-------|
| **Problema** | Servidor HTTP puro, sem suporte a TLS. Nenhuma opção de certificado, key ou configuração SSL. |
| **Impacto** | Todas as comunicações (incluindo tokens Bearer, dados de notas, comandos admin) trafegam em texto puro. MITM total. |
| **Evidência** | `src/bin/server.rs:118`: `apexstore::api::start_server(engine, server_config)` — usa `HttpServer::new()`, não `HttpServer::bind_rustls()`. Nenhuma dependência `rustls` ou `openssl` no `Cargo.toml`. |
| **Recomendação** | Adicionar suporte a TLS nativo (rustls ou openssl via actix-web). Pelo menos documentar deploy atrás de reverse proxy (nginx/caddy) com requisito obrigatório. |
| **Validação** | Verificar que `curl -k https://localhost:8080/health/liveness` funciona ou que docs exigem reverse proxy. |
| **Prioridade** | Imediata |

### C-05: CDC Webhook sem Autenticação nem Retry

| Campo | Valor |
|-------|-------|
| **Problema** | `WebhookPublisher::publish` no `cdc.rs` faz POST do evento para o endpoint configurado sem qualquer autenticação (Bearer token, HMAC, etc.) e sem retry em caso de falha. Dados sensíveis vazam para qualquer endpoint configurado ou interceptado. |
| **Impacto** | Se um atacante conseguir alterar a variável `CDC_ENDPOINT` (via env injection, config map, etc.), todas as mutações de dados são enviadas para ele. Sem retry, eventos são perdidos em falha de rede. |
| **Evidência** | `src/infra/cdc.rs:217-250`: função `publish` faz `reqwest::Client::post().json().send().await` sem `Authorization` header, sem retry, sem timeout explícito. |
| **Recomendação** | Adicionar `Authorization` header configurável no `CdcConfig`. Envolver envio em `RetryConfig::default()`. Adicionar timeout de 5s. Usar circuit breaker. |
| **Validação** | Teste unitário: mock HTTP server que verifica header `Authorization` no POST de CDC. |
| **Prioridade** | Alta |

---

## 🟠 Achados Altos

### H-01: N+1 Query no Endpoint `/scan`

| Campo | Valor |
|-------|-------|
| **Problema** | `GET /scan` primeiro lista todas as keys via `engine.keys()`, depois faz uma `get_cf` para cada key individualmente. Isso é 1 + N queries. |
| **Impacto** | Com 10.000 keys, são 10.001 chamadas ao engine. Latência O(N) em vez de scan eficiente. OOM ao carregar todas as keys em memória ao mesmo tempo. |
| **Evidência** | `src/api/mod.rs:439-463`: `for k in keys { engine.get_cf("default", ...) }` |
| **Recomendação** | Usar `engine.scan()` (prefix scan com iterator) que já existe no engine e retorna key+value em uma única passagem. |
| **Validação** | `cargo bench --bench scan_bench` — comparar latência antes/depois. |
| **Prioridade** | Alta |

### H-02: Batch Insert sem Limite de Tamanho

| Campo | Valor |
|-------|-------|
| **Problema** | `POST /keys/batch` aceita `Vec<FrontendSetBody>` sem limite de tamanho. Atacante pode enviar 100.000 registros em uma única requisição. |
| **Impacto** | DoS via batch gigante. Engine LSM-tree faz `put_cf` para cada registro individualmente (sem batch transaction). Amplificação de escrita. |
| **Evidência** | `src/api/mod.rs:406-431`: sem `.take()` ou `max_batch` validation. |
| **Recomendação** | Adicionar `max_batch: usize` configurável (default 1000). Validar no middleware. Usar transação atômica do engine se disponível. |
| **Validação** | Teste: `POST /keys/batch` com 10001 registros → deve retornar 413. |
| **Prioridade** | Alta |

### H-03: GraphQL Playground Acessível em Produção

| Campo | Valor |
|-------|-------|
| **Problema** | `GET /graphql/playground` e `GET /graphql` retornam o Playground interativo (GraphQL IDE). Qualquer um pode executar queries arbitrárias. |
| **Impacto** | Ferramenta de desenvolvimento exposta em produção. Atacante pode explorar schema GraphQL, fazer queries profundas, enumerar dados. |
| **Evidência** | `src/api/mod.rs:491-493`: playground registrado sem verificação de ambiente. |
| **Recomendação** | Desabilitar playground quando `env!("PROFILE") != "debug"` ou via config `graphql_playground_enabled: bool`. |
| **Validação** | Teste: `GET /graphql/playground` em modo release → deve retornar 404. |
| **Prioridade** | Alta |

### H-04: WebSocket Sync sem Autenticação

| Campo | Valor |
|-------|-------|
| **Problema** | `GET /ws/sync` usa `actix-ws` sem middleware de autenticação. O `sync_handler` não verifica token. |
| **Impacto** | Qualquer um pode abrir WebSocket, assinar notas, receber mudanças em tempo real e enviar alterações. Vazamento de dados via push. |
| **Evidência** | `src/api/sync.rs:187-211`: handler `sync_handler` sem `require_permission`. |
| **Recomendação** | Adicionar validação de token no handshake WebSocket. Extrair token do query param `?token=...` ou header `Sec-WebSocket-Protocol`. |
| **Validação** | Teste: conectar WebSocket sem token → deve fechar conexão com 4001. |
| **Prioridade** | Alta |

### H-05: Rate Limiter Usa Mutex Simples (Gargalo)

| Campo | Valor |
|-------|-------|
| **Problema** | `RateLimiterState` usa `Mutex<HashMap<IpAddr, IpTrack>>`. Cada requisição adquire o lock global para verificar rate limit. Sob alta concorrência, o mutex se torna gargalo. |
| **Impacto** | Em benchmarks de 500+ conexões concorrentes, o rate limiter pode aumentar latência em 10-50ms por requisição devido à contenção de lock. |
| **Evidência** | `src/api/rate_limiter.rs:50`: `requests: Mutex<HashMap<IpAddr, IpTrack>>` |
| **Recomendação** | Usar sharded Mutex (256 shards by hash do IP), ou implementar sliding window com `DashMap` (dashmap crate), ou usar algoritmo token bucket lock-free com `AtomicU64`. |
| **Validação** | `cargo bench --bench latency_bench` — comparar P99 latency antes/depois com 1000 conexões. |
| **Prioridade** | Alta |

### H-06: Idempotency Middleware Não Integrado

| Campo | Valor |
|-------|-------|
| **Problema** | `IdempotencyMiddleware` existe em `src/infra/idempotency.rs` mas **nunca é instanciado nem registrado** na cadeia de middleware do actix-web (`src/api/mod.rs:569-588`). |
| **Impacto** | Retry de clientes pode causar duplicação de writes. Se um PUT /keys/key for executado duas vezes, o segundo pode sobrescrever dados já atualizados (dependendo do timestamp). |
| **Evidência** | `src/api/mod.rs:569-588`: middleware list inclui `RequestTimeout`, `RateLimiter`, `AccessControl`, `Logger`, `Cors`, `HttpAuthentication` — mas não `IdempotencyMiddleware`. |
| **Recomendação** | Integrar `IdempotencyMiddleware` como middleware global para mutações (PUT, POST, DELETE). Extrair `Idempotency-Key` header. |
| **Validação** | Teste de integração: enviar `PUT /keys/k` com `Idempotency-Key: xyz` duas vezes → mesma resposta. |
| **Prioridade** | Alta |

### H-07: Variáveis de Ambiente Sem Validação

| Campo | Valor |
|-------|-------|
| **Problema** | `ServerConfig::from_env()` faz parse de variáveis com `unwrap_or(default)` genérico. Valores maliciosos ou inválidos são silenciosamente trocados pelo default sem warning. |
| **Impacto** | Se `WORKERS=-1` for injetado, silenciosamente vira `workers: None` (auto). Se `PORT=0` (porta aleatória), servidor pode ligar em porta inesperada. Falta de validação de range. |
| **Evidência** | `src/api/config.rs:77-170`: todos os `parse::<T>()` usam `unwrap_or(default_value)` sem log de warning. |
| **Recomendação** | Adicionar logging warning quando env var é inválida. Validar ranges (porta 1-65535, workers 1-64, etc.). |
| **Validação** | Teste: `WORKERS=-1 cargo run` → log warning no startup. `WORKERS=0` → erro de validação. |
| **Prioridade** | Alta |

### H-08: Scan de Keys Carrega Tudo em Memória

| Campo | Valor |
|-------|-------|
| **Problema** | `GET /keys` (sem prefix) chama `engine.keys()` que retorna **todas as keys** do banco em um `Vec<Vec<u8>>`. Para bancos com milhões de keys, isso causa OOM. |
| **Impacto** | Com 5M de keys de 32 bytes = 160MB só de keys em memória. Antes de serializar JSON. |
| **Evidência** | `src/api/mod.rs:156-163`: `engine.keys()?.into_iter().take(limit)` — `take` limita output mas o vetor inteiro já foi alocado. |
| **Recomendação** | Implementar `engine.keys_with_limit(limit)` que usa inner iterator e para após `limit` itens. Ou modificar `search_prefix` com prefixo vazio. |
| **Validação** | Teste com 1M keys: consumo de memória < 50MB. |
| **Prioridade** | Alta |

---

## 🟡 Achados Médios

### M-01: Workers Ilimitados (Auto = CPU Cores)

| Campo | Valor |
|-------|-------|
| **Problema** | `WORKERS` vazio = auto-detect. Em servidores com muitos cores (32, 64), isso cria dezenas de workers que competem pelo lock do engine (`Mutex` no core). |
| **Impacto** | Contenção alta no lock do engine (cada operação adquire `parking_lot::Mutex` no core). Workers extras gastam CPU em spinning. |
| **Evidência** | `src/bin/server.rs:594-596`: `if let Some(workers) = config.workers { server_builder = server_builder.workers(workers); }` |
| **Recomendação** | Recomendar default de 4 workers. Documentar que engine lock-bound. Adicionar warning se `workers > 8`. |
| **Validação** | `cargo bench --bench mixed_bench` com 4 vs 32 workers. |
| **Prioridade** | Média |

### M-02: Ausência de Proteção CSRF

| Campo | Valor |
|-------|-------|
| **Problema** | Nenhum token CSRF, SameSite cookie, ou verificação de origem/Referer nas mutações. Embora actix-web exija `Content-Type: application/json` para `web::Json`, navegadores não enviam automaticamente. |
| **Impacto** | Se auth estivesse habilitado com cookie-based (não é o caso atual, mas arquiteturalmente), CSRF seria possível. |
| **Evidência** | Todo o `src/api/mod.rs` — nenhuma verificação de `Origin`, `Referer`, ou token CSRF. |
| **Recomendação** | Adicionar middleware de verificação de origem para mutações. Exigir header `X-CSRF-Token` ou `Origin` check. |
| **Validação** | Teste: POST sem `Origin` header → 403. |
| **Prioridade** | Média |

### M-03: Token Timing Attack Parcial

| Campo | Valor |
|-------|-------|
| **Problema** | `constant_time_compare` compara strings hex (SHA-256) byte a byte, o que é constant-time. Porém, a comparação de tamanho (`a.len() != b.len()`) vaza informação de tamanho do hash. |
| **Impacto** | Extremamente baixo em prática (atacante precisaria de >10^6 requisições para extrair 64 chars). Mas quebra a premissa de constant-time. |
| **Evidência** | `src/api/auth/token.rs:131-138`: `if a.len() != b.len() { return false; }` |
| **Recomendação** | Usar crate `subtle` ou `constant_time_eq` para comparação de bytes. Hash tokens armazenados como `[u8; 32]` em vez de hex string. |
| **Validação** | Teste: `validate_token` com tokens de diferentes tamanhos deve levar o mesmo tempo (±5%). |
| **Prioridade** | Média |

### M-04: Backup sem Criptografia

| Campo | Valor |
|-------|-------|
| **Problema** | `BackupScheduler::backup_now()` copia snapshots para diretório de backup sem qualquer criptografia. Dados sensíveis ficam em texto puro no disco. |
| **Impacto** | Se o diretório de backup for comprometido, todos os dados são expostos. Backups em S3/NFS sem criptografia violam compliance (GDPR, HIPAA, SOC2). |
| **Evidência** | `src/infra/backup_scheduler.rs:170-208`: nenhuma chamada a `encrypt_block` ou similar. Backup é cópia literal dos arquivos SSTable. |
| **Recomendação** | Integrar `Encryptor` no backup. Usar AES-256-GCM com chave separada (rotação periódica). |
| **Validação** | Teste: backup → arquivos .encrypted. Tentar ler sem chave → dados ininteligíveis. |
| **Prioridade** | Média |

### M-05: Compactação Usa `std::thread::sleep` Dentro de Lock

| Campo | Valor |
|-------|-------|
| **Problema** | (Verificar engine internals) O `retry_with_backoff` usa `std::thread::sleep(Duration::from_millis(...))` que bloqueia a thread atual. Se chamado dentro de um lock no compaction, bloqueia outros workers. |
| **Impacto** | Thread pool bloqueada durante sleep de até 5s (max_delay_ms). Redução de throughput. |
| **Evidência** | `src/infra/retry.rs:102`: `std::thread::sleep(Duration::from_millis(actual_delay_ms))` |
| **Recomendação** | Usar `tokio::time::sleep` para async contexts. Separar retry de blocking IO. |
| **Validação** | Teste de estresse: 1000 writes concorrentes durante compactação → sem starvation. |
| **Prioridade** | Média |

### M-06: CDC Publisher sem Timeout

| Campo | Valor |
|-------|-------|
| **Problema** | `WebhookPublisher::publish` não especifica timeout na requisição HTTP. Se o endpoint CDC travar, a goroutine fica bloqueada indefinidamente. |
| **Impacto** | Escrita no engine fica bloqueada aguardando CDC. Degeneração para completa indisponibilidade. |
| **Evidência** | `src/infra/cdc.rs`: reqwest post sem `.timeout()`. |
| **Recomendação** | Adicionar `reqwest::Client::builder().timeout(Duration::from_secs(5))`. Usar circuit breaker para CDC. |
| **Validação** | Teste: configurar CDC endpoint que não responde → timeout após 5s. |
| **Prioridade** | Média |

### M-07: Falta de Auditoria (Access Log)

| Campo | Valor |
|-------|-------|
| **Problema** | O logger `actix_web::middleware::Logger::default()` registra apenas método, path, status, duração. **Não registra**: quem fez a requisição (principal), o recurso acessado (key), o resultado detalhado. |
| **Impacto** | Impossível fazer auditoria forense: quem deletou qual key, quando. |
| **Evidência** | `src/api/mod.rs:574`: `.wrap(actix_web::middleware::Logger::default())` — formato de log não customizado. |
| **Recomendação** | Customizar `Logger` para incluir `X-User-Id` (do token), `X-Request-Id`, key do path, response status. Adicionar log estruturado em formato JSON. |
| **Validação** | Verificar logs: `{"method":"DELETE","path":"/keys/secret","user":"alice","status":200}` |
| **Prioridade** | Média |

### M-08: Degradation Manager Não Integrado com API

| Campo | Valor |
|-------|-------|
| **Problema** | `DegradationManager` existe mas não é verificado nos handlers da API. Writes continuam mesmo quando o disco está crítico. |
| **Impacto** | Escrita em disco cheio causa corrupção de SSTable e perda de dados. |
| **Evidência** | `src/infra/degradation.rs` — implementação completa mas não há chamada a `check_write_allowed()` nos handlers `put_key`, `post_key`, `delete_key`, `batch_keys`. |
| **Recomendação** | Integrar `DegradationManager::check_write_allowed()` em todos os handlers de mutação. Conectar `DiskMonitor::on_critical` para setar ReadOnly. |
| **Validação** | Teste: encher disco → POST /keys → retorna 503 Service Unavailable. |
| **Prioridade** | Média |

### M-09: Sem Limite de Conexões por IP

| Campo | Valor |
|-------|-------|
| **Problema** | Rate limiter é baseado em requisições/minuto, não em conexões simultâneas. `max_connections=10000` é global, não por IP. |
| **Impacto** | Um único IP pode abrir 10000 conexões simultâneas e exaurir o file descriptor pool. |
| **Evidência** | `src/api/rate_limiter.rs`: rastreia requisições em janela de 60s, mas não limita conexões simultâneas por IP. |
| **Recomendação** | Adicionar `max_connections_per_ip: usize` (default 100). Usar `HashMap<IpAddr, AtomicUsize>` para tracking de conexões ativas. |
| **Validação** | Teste: 200 conexões simultâneas do mesmo IP → 100 aceitas, 100 rejeitadas com 429. |
| **Prioridade** | Média |

### M-10: Dependência Bincode UNMAINTAINED

| Campo | Valor |
|-------|-------|
| **Problema** | `bincode` 1.3.3 é marcado como UNMAINTAINED (RUSTSEC-2025-0141), mas está como dependência (aparentemente removida — verificar Cargo.toml atual). |
| **Impacto** | Se presente, bugs de segurança não serão corrigidos. |
| **Evidência** | `SECURITY_REPORT.md:64` — mas não aparece no `Cargo.toml` atual (`postcard` substituiu?). Verificar `Cargo.lock`. |
| **Recomendação** | Confirmar que `bincode` não está em nenhuma dependência transitiva. Executar `cargo audit` regularmente. |
| **Validação** | `cargo audit` não deve reportar `bincode`. |
| **Prioridade** | Média |

---

## 🔵 Achados Baixos

### L-01: CLI sem Gestão de Tokens

| Campo | Valor |
|-------|-------|
| **Problema** | CLI (`apexstore-cli`) não tem comandos para criar, listar, revogar tokens de API. |
| **Impacto** | Administradores precisam criar tokens manualmente via API `POST /admin/tokens` (que nem existe como endpoint documentado). |
| **Evidência** | `src/bin/cli.rs`: comandos get/put/delete/scan — sem subcomando `token`. |
| **Recomendação** | Adicionar `apexstore-cli token create/list/revoke` com persistência no engine. |
| **Validação** | `apexstore-cli token create --name "ci-user" --permission read` → retorna token. |
| **Prioridade** | Baixa |

### L-02: Admin Dashboard Auto-Refresh sem Cache Bust

| Campo | Valor |
|-------|-------|
| **Problema** | Dashboard HTML auto-refresh a cada 5s via `location.reload()`, não via `fetch()`. Causa flash visual e perde estado. |
| **Impacto** | Experiência de monitoração ruim. Pequena sobrecarga de CPU/rendering. |
| **Evidência** | `src/api/admin/dashboard.rs:216`: `setInterval(updateTime, 1000); setTimeout(function() { location.reload(); }, 5000);` |
| **Recomendação** | Substituir por `fetch('/stats/all')` com atualização parcial do DOM. |
| **Validação** | Dashboard atualiza sem flash visual. |
| **Prioridade** | Baixa |

### L-03: Sem `cargo audit` no CI

| Campo | Valor |
|-------|-------|
| **Problema** | Workflow CI (`ci.yml`) não executa `cargo audit` para detectar vulnerabilidades em dependências. |
| **Impacto** | Vulnerabilidades em dependências só são descobertas manualmente. |
| **Evidência** | `SECURITY_REPORT.md:105`: `#183 — No cargo audit in CI`. Workflow não tem step de audit. |
| **Recomendação** | Adicionar `cargo audit` ao CI. Usar `cargo-audit` com `--deny warnings`. |
| **Validação** | CI passa com `cargo audit`. |
| **Prioridade** | Baixa |

### L-04: Endpoints Frontend Duplicados

| Campo | Valor |
|-------|-------|
| **Problema** | `POST /keys` (frontend) e `PUT /keys/{key}` fazem a mesma coisa com APIs diferentes. `GET /stats/all` e `GET /stats` são redundantes. |
| **Impacto** | Superfície de ataque maior sem necessidade. Manutenção duplicada. |
| **Evidência** | `src/api/mod.rs:345-369` (post_key) vs `73-99` (put_key). `319-342` (get_stats_all) vs `193-217` (get_stats). |
| **Recomendação** | Consolidar: remover endpoints duplicados ou fazê-los redirecionar. Versionar API (`/v1/keys`). |
| **Validação** | Testes existentes continuam passando após remoção dos duplicados. |
| **Prioridade** | Baixa |

### L-05: Sem Health Check de Dependências Externas

| Campo | Valor |
|-------|-------|
| **Problema** | Health checks (`/health/liveness`, `/health/readiness`) verificam apenas o engine local, não dependências externas (CDC endpoint, Webhook, OTLP collector). |
| **Impacto** | Readiness retorna 200 mesmo quando todas as dependências externas estão down. K8s não mata pod. |
| **Evidência** | `src/api/health.rs`: liveness e readiness só verificam engine stats. |
| **Recomendação** | Adicionar health checks para dependências: CDC endpoint (ping), OTLP (gRPC health check), disk space. |
| **Validação** | CDC endpoint retornando 503 → `/health/readiness` retorna 503. |
| **Prioridade** | Baixa |

---

## 🗄️ Riscos do Banco de Dados (LSM-tree)

### DB-01: Write Amplification sem Monitoramento (Médio)

| Problema | O LSM-tree tem write amplification inerente (cada write é reescrito múltiplas vezes na compactação). `write_amplification.rs` bench existe mas não há métrica exposta no Prometheus. |
|----------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Impacto | Sem monitorar write amplification, usuários podem configurar tamanhos de nível errados e ter 20-50x write amplification, reduzindo vida de SSD. |
| Recomendação | Adicionar métrica `apexstore_write_amplification_ratio` ao Prometheus. Basear em `compaction_bytes_written / user_bytes_written`. |

### DB-02: Read Amplification em GET sem Bloom Filter (Médio)

| Problema | Se o Bloom Filter não for configurado (ou `BLOOM_FALSE_POSITIVE_RATE` muito alto), cada `GET` precisa verificar todos os níveis (L0..Ln). Com 10 níveis, cada GET faz até 10 reads de SSTable. |
|----------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Impacto | Latência P99 pode disparar de 1ms para 50ms em bancos com muitos níveis e sem bloom filter efetivo. |
| Recomendação | Exigir Bloom Filter (default 1% false positive). Monitorar `bloom_filter_negatives_total` para detectar filtros ineficazes. |

### DB-03: Compaction Pode Causar Write Stalls (Baixo)

| Problema | Se a taxa de escrita excede a capacidade de compactação, o memtable enche e o engine bloqueia writes. `CompactionBackpressure` existe mas não está integrado com o rate limiter da API. |
|----------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Impacto | Writes bloqueados por segundos. Picos de latência. |
| Recomendação | Integrar `CompactionBackpressure` com `RateLimiterState` da API. Quando backpressure > threshold, reduzir rate limit da API dinamicamente. |

### DB-04: Sem Range Deletions Otimizadas (Baixo)

| Problema | `delete_cf` deleta uma chave por vez. Não há suporte a range delete (`delete_range(start, end)`) que eliminaria toda uma faixa com uma única operação lógica. |
|----------|--------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Impacto | Deletar 1M keys com prefixo requer 1M operações. Write stall. |
| Recomendação | Implementar `delete_range()` no engine com tombstone de range (já há `range_start`/`range_end` no `LogRecord`). |

### DB-05: Block Cache Tamanho Fixo (Baixo)

| Problema | `GlobalBlockCache::new(64, 4096)` — tamanho fixo de 64MB. Sem resize dinâmico ou adaptive cache. |
|----------|--------------------------------------------------------------------------------------------------|
| Impacto | Cache pode estar superdimensionado (desperdício de memória) ou subdimensionado (cache miss rate alto). |
| Recomendação | Monitorar `cache_hits_total / (cache_hits_total + cache_misses_total)`. Adicionar resize via API admin. |

---

## 📋 Plano de Correção

### Fase 1 — Imediata (Semanas 1-2)

| Prioridade | ID | Ação | Esforço |
|-----------|----|------|---------|
| 🔴 P0 | C-01 | Mudar default auth para `true`. Adicionar middleware obrigatório. | 2h |
| 🔴 P0 | C-02 | Remover `Cors::permissive()`. Exigir origens explícitas. | 1h |
| 🔴 P0 | C-03 | Reduzir payload max para 1MB default. Adicionar `json_payload` middleware. | 1h |
| 🔴 P0 | C-04 | Adicionar suporte TLS (rustls) ou documentar reverse proxy obrigatório. | 8h |
| 🟠 P1 | H-01 | Refatorar `/scan` para usar `engine.scan()` em vez de N+1 queries. | 3h |
| 🟠 P1 | H-02 | Adicionar `max_batch_size` limit (default 1000). | 1h |

### Fase 2 — Curto Prazo (Semanas 3-4)

| Prioridade | ID | Ação | Esforço |
|-----------|----|------|---------|
| 🟠 P1 | H-03 | Desabilitar GraphQL playground em produção. | 1h |
| 🟠 P1 | H-04 | Adicionar auth no WebSocket sync. | 4h |
| 🟠 P1 | H-05 | Substituir `Mutex<HashMap>` por sharded ou lock-free rate limiter. | 6h |
| 🟠 P1 | H-06 | Integrar `IdempotencyMiddleware` na cadeia de middleware. | 3h |
| 🟠 P1 | H-07 | Adicionar validação de env vars com logging de warning. | 2h |
| 🟠 P1 | H-08 | Implementar `keys_with_limit()` para evitar OOM em scans. | 4h |

### Fase 3 — Médio Prazo (Semanas 5-6)

| Prioridade | ID | Ação | Esforço |
|-----------|----|------|---------|
| 🔴 P0 | C-05 | Adicionar auth + retry + timeout no CDC Webhook. | 4h |
| 🟡 P2 | M-01 | Fixar default workers=4. Documentar engine lock-bound. | 1h |
| 🟡 P2 | M-02 | Adicionar CSRF protection middleware. | 3h |
| 🟡 P2 | M-03 | Usar `subtle::ConstantTimeEq` para token comparison. | 1h |
| 🟡 P2 | M-04 | Criptografar backups com AES-256-GCM. | 4h |
| 🟡 P2 | M-06 | Adicionar timeout no CDC HTTP client. | 1h |
| 🟡 P2 | M-07 | Customizar Logger para auditoria (principal, path, status). | 2h |
| 🟡 P2 | M-08 | Integrar DegradationManager nos handlers de mutação. | 3h |
| 🟡 P2 | M-09 | Adicionar `max_connections_per_ip`. | 3h |

### Fase 4 — Contínuo (Sprints)

| Prioridade | ID | Ação | Esforço |
|-----------|----|------|---------|
| 🔵 P3 | L-01..L-05 | Melhorias de CLI, dashboard, CI audit, consolidar endpoints, health checks | 12h |
| 🟡 P2 | M-05 | Refatorar retry para async com tokio::time::sleep | 3h |
| 🟡 P2 | M-10 | Confirmar remoção de bincode, adicionar `cargo audit` ao CI | 1h |

---

## 🧪 Plano de Testes

### Testes de Segurança

| Teste | Ferramenta | Critério |
|-------|-----------|----------|
| Auth bypass | curl / httpx | `GET /keys` sem token → 401 |
| CORS reflection | curl -H "Origin: https://evil.com" | Response não deve refletir `Origin` |
| Payload oversized | curl -X PUT -d @50MB.json | 413 Payload Too Large |
| Path traversal | GET /keys/../../../etc/passwd | 404 (não 200 com conteúdo) |
| Rate limit bypass | 101 req/min de mesmo IP | 429 na 101ª |
| Token fuzzing | 1000 tokens aleatórios | 401 para todos |
| WebSocket auth | ws sem token | Conexão rejeitada |
| GraphQL iD | query `{__schema{types{name}}}` com profundidade 10 | Limitada a 3 níveis |
| Batch DoS | POST /keys/batch com 10001 registros | 413 ou 429 |
| CDC injection | Configurar CDC_ENDPOINT para servidor do atacante | Dados não devem vazar |
| TLS check | curl sem `-k` | Handshake TLS bem-sucedido |

### Testes de Resiliência

| Teste | Cenário | Critério |
|-------|---------|----------|
| Kill + recovery | Matar processo, reiniciar | WAL recovery, dados íntegros |
| Disk fill | Preencher disco até threshold | Engine entra em ReadOnly |
| Conexões em massa | 2000 conexões simultâneas | 2000 processadas, demais rejeitadas ordeiramente |
| Compactação + writes | 1000 writes/s durante compactação | Writes não bloqueiam > 1s |
| CDC offline | Endpoint CDC fora do ar | Engine continua operacional |
| Panic em worker | Forçar panic em thread de compactação | Thread reinicia, engine continua |

### Testes de Performance

| Teste | Métrica | Alvo |
|-------|---------|------|
| GET latency (P50/P99/P999) | `latency_bench` | < 500µs / < 5ms / < 50ms |
| PUT latency (P50/P99) | `latency_bench` | < 1ms / < 10ms |
| Scan throughput (1000 keys) | `scan_bench` | > 1000 keys/s |
| Mixed workload | `mixed_bench` | > 5000 ops/s |
| Write amplification | `write_amplification` | < 10x |
| Concurrent connections | `stress_bench` | > 500 conexões simultâneas sem erro |

---

## ✅ Checklist Final para Produção

### Configuração Obrigatória

- [ ] `API_AUTH_ENABLED=true` — autenticação ativa
- [ ] Token de admin criado via `POST /admin/tokens` (ou CLI)
- [ ] `CORS_ORIGINS` configurado com origens específicas (nunca vazio)
- [ ] `MAX_JSON_PAYLOAD_SIZE` reduzido para 1MB
- [ ] Deploy atrás de TLS (nginx, caddy, ou TLS nativo)
- [ ] `REQUEST_TIMEOUT_SECONDS` configurado (30s default)
- [ ] `RATE_LIMIT_REQUESTS_PER_MINUTE` configurado (100 default)

### Configuração Recomendada

- [ ] `WORKERS=4` (não auto)
- [ ] `BLOCK_CACHE_SIZE_MB=256` (ou mais, dependendo da carga)
- [ ] `BLOOM_FALSE_POSITIVE_RATE=0.01` (1%)
- [ ] `PREFIX_COMPRESSION_ENABLED=true` (se keys têm prefixos comuns)
- [ ] `WAL_ARCHIVE_ENABLED=true` com `WAL_MAX_SIZE=67108864` (64MB)
- [ ] `BACKUP_SCHEDULER` configurado com `retention_count >= 10`

### Monitoramento

- [ ] Prometheus `/metrics` integrado ao scraping
- [ ] OTLP export configurado (`OTEL_EXPORTER_OTLP_ENDPOINT`)
- [ ] Dashboard de latency P50/P99/P95 configurado
- [ ] Alerta de `apexstore_errors_total > 0` em 5 min
- [ ] Alerta de `apexstore_cache_hits_total < 0.8 * apexstore_gets_total` (hit rate < 80%)
- [ ] Alerta de `disk_available_bytes < warn_threshold` (1GB)

### Pipeline CI/CD

- [ ] `cargo audit` executado em cada PR
- [ ] `cargo clippy -- -D warnings` passa
- [ ] `cargo test --all-features` passa
- [ ] `cargo bench` sem regressão > 10%
- [ ] `cargo fmt --check` passa

### Documentação

- [ ] `docs/SECURITY.md` atualizado com procedimentos de incidentes
- [ ] `docs/OPS.md` com runbooks de recovery
- [ ] `.env.example` com valores seguros (auth=true, CORS configurado)
- [ ] Variáveis de produção documentadas

---

## Suposições Explicitas

1. **Auth stateless**: Usei tokens Bearer armazenados no engine como única fonte de verdade. Não há OAuth2, OIDC, SAML ou provedor externo.
2. **Deployment**: Assumo deploy em container (Docker/K8s) com reverse proxy (nginx). Recomendo fortemente TLS no proxy.
3. **Monitoração**: Assumo Prometheus + Grafana para métricas, e OTLP collector opcional para tracing.
4. **Backup**: Assumo backup periódico via `BackupScheduler` para diretório local ou volume montado (NFS/S3 via CSI).
5. **Dados sensíveis**: Considerei notas, tags e values como dados sensíveis que precisam de criptografia em repouso e em trânsito.
6. **Escala**: Análise considera até 10M keys, 1000 writes/s, 10000 conexões simultâneas.
7. **Regulatory**: Não assumi compliance específico (LGPD/GDPR/HIPAA), mas recomendações seguem boas práticas gerais.

---

## Apêndice: Arquivos Analisados

```
src/api/mod.rs                 → Handlers, rotas, configuração CORS
src/api/auth/*                 → Token, middleware, manager, errors
src/api/config.rs              → Configuração do servidor
src/api/health.rs              → Health checks
src/api/rate_limiter.rs        → Rate limiting (IP-based)
src/api/access_control.rs      → Access control middleware
src/api/timeout_middleware.rs  → Timeout middleware
src/api/admin/*                → Dashboard, configuração admin
src/api/sync.rs                → WebSocket sync
src/api/graphql/mod.rs         → GraphQL schema e playground
src/bin/server.rs              → Entrypoint do servidor
src/infra/retry.rs             → Retry com backoff
src/infra/circuit_breaker.rs   → Circuit breaker pattern
src/infra/idempotency.rs       → Idempotency middleware (não integrado)
src/infra/backpressure.rs      → Compaction backpressure
src/infra/backup_scheduler.rs  → Backup automático
src/infra/telemetry.rs         → OpenTelemetry tracing + metrics
src/infra/metrics.rs           → Engine metrics (Prometheus + OTel)
src/infra/panic_recovery.rs    → Panic recovery
src/infra/disk_monitor.rs      → Disk space monitor
src/infra/scrubber.rs          → Data integrity scrubber
src/infra/degradation.rs       → Degradation modes
src/infra/cdc.rs               → Change Data Capture
src/infra/error.rs             → Error types (LsmError)
src/infra/memory_limiter.rs    → Memory budget
src/infra/watchdog.rs          → Health watchdog
src/infra/access_control.rs    → Policy engine
src/storage/encryption.rs      → AES-256-GCM encryption at rest
src/storage/wal.rs             → Write-Ahead Log
src/storage/blob_store.rs      → Blob storage
src/core/engine/mod.rs         → Core engine (amostra)
Cargo.toml                     → Dependências
.env.example                   → Configuração de ambiente
SECURITY_REPORT.md             → Relatório de segurança anterior
```

---

*Auditoria gerada em 2026-05-26. Revisar a cada 6 meses ou após mudanças significativas na stack.*
