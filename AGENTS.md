# AGENTS.md

<!-- Este arquivo é gerado automaticamente pelo boilerplate-opencode. -->
<!-- Não edite manualmente a seção entre as tags AUTO-GENERATED. -->
<!-- Preencha as seções marcadas com > após a instalação. -->

## Projeto

**ApexStore** é um storage engine LSM-tree embarcado de alta performance em Rust, com formato SSTable V2, compressão LZ4 e Bloom Filters. Oferece API HTTP (actix-web), CLI e TUI (ratatui). Ideal para aplicações que precisam de key-value store embarcada com baixa latência e alta throughput.

## Stack

- **Linguagem:** Rust 2021 edition
- **Framework Web:** actix-web 4 (API REST)
- **TUI:** ratatui + crossterm
- **Formato de armazenamento:** SSTable V2 com LZ4, CRC32, Bloom Filters
- **Infra:** GitHub Actions (CI/CD), Docker (opcional)
- **Ferramentas:** cargo (build, test, clippy, fmt, doc, audit)

<!-- AUTO-GENERATED:START -->
## Regras gerais

### Commits
- Seguir Conventional Commits: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`
- Mensagens em inglês, imperativo presente: "add feature" não "added feature"
- Commits atômicos: uma responsabilidade por commit

### Pull Requests
- Título segue Conventional Commits
- Descrição inclui: o que foi feito, por que, como testar
- PR sem testes não é mergeada

### Código
- Sem código morto ou comentado
- Sem debugging esquecido (`console.log`, `fmt.Println`, `print()`)
- Sem secrets no código
- Tratamento de erros obrigatório

### CI/CD
- Pipeline deve passar antes do merge
- Jobs locais devem ser validados com `workflow-agent` antes de abrir PR
- Arquivo `.task-state.json` deve estar limpo após conclusão da tarefa

## Regras de CI/CD

### workflow-agent
O script `scripts/workflow-agent.ts` executa localmente os jobs do `ci.yml` que não dependem de secrets externos.

Saída JSON linha a linha:
- `job_started` — início de um job
- `step_started` / `step_finished` — início/fim de cada step com `exitCode`
- `job_finished` — status do job (`success` | `failed` | `skipped`)
- `workflow_finished` — resultado final

Jobs pulados automaticamente quando requerem secrets externos:
- `secrets-scan`, `semgrep`, `sonarcloud` e similares

### check-todos
O script `scripts/check-todos.ts` verifica se os arquivos listados nos TODOs do `.task-state.json` existem.

Saída JSON: `{ ok: boolean, totals: {...}, results: [...] }`

### Pré-requisitos locais
- Docker disponível no PATH
- Node.js ≥ 20
- `cd scripts && npm install`

## Regras Rust

### Convenções
- `clippy` obrigatório: `cargo clippy -- -D warnings`
- `rustfmt` para formatação
- Sem `unwrap()` em código de produção; usar `?` ou tratamento explícito
- Prefer `thiserror` para erros de biblioteca, `anyhow` para binários
- Lifetimes explícitos quando necessário, não por padrão

### Testes
- Unit tests no mesmo arquivo com `#[cfg(test)]`
- Integration tests em `tests/`
- `cargo test --all-features`

### Build e ferramentas
```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo audit
```
<!-- AUTO-GENERATED:END -->

## Comandos úteis

```bash
# Instalar dependências (scripts CI)
cd scripts && npm install

# Build (release)
cargo build --release

# Testes
cargo test --all-features --workspace

# Lint (clippy)
cargo clippy --all-targets --all-features -- -D warnings

# Formatação
cargo fmt --all -- --check

# Documentação
cargo doc --no-deps --all-features

# Auditoria de segurança
cargo audit

# Pipeline CI local
npx tsx scripts/workflow-agent.ts .github/workflows/ci.yml

# Verificar TODOs
npx tsx scripts/check-todos.ts .task-state.json

# Benchmarks
cargo bench
```

## Convenções

- **Commits**: Conventional Commits (`feat`, `fix`, `chore`, `docs`, `refactor`)
- **Branches**: `feat/<slug>`, `fix/<slug>`, `chore/<slug>`
- **Naming**: snake_case para variáveis/funções, PascalCase para tipos/structs/enums
- **Testes**: arquivos `*.test.ts` ao lado do módulo testado
- **Estrutura de pastas**: 
  - `src/` — código fonte (lib, bin, core, api, cli, tui)
  - `benches/` — benchmarks criterion
  - `tests/` — testes de integração
  - `scripts/` — utilitários (CI, formatação)
  - `.github/` — workflows e actions

## Contexto de domínio

- **LSM-Tree**: Estrutura de dados Log-Structured Merge-Tree. Escritas vão para um memtable (WAL + skiplist), flush em SSTables no nível L0, e compaction periódica mergeia níveis inferiores.
- **SSTable V2**: Formato de arquivo sorted string table com header (magic, version), bloom filter, blocks de dados indexados, e trailer com metadados e CRC32.
- **Memtable**: Buffer em memória (WAL + skiplist) que acumula escritas antes de flush para SSTable.
- **Compaction**: Processo de merge de SSTables de níveis inferiores para manter a estrutura em árvore e limitar amplificação de leitura/escrita.
- **Bloom Filter**: Filtro probabilístico que acelera buscas evitando ler SSTables que não contêm a chave.
- **WAL (Write-Ahead Log)**: Log de escrita antecipada para garantir durabilidade e recovery.
- **Block Cache**: Cache LRU de blocos de dados desserializados para acelerar leituras repetidas.
- **MergeIterator**: Iterador que mergeia múltiplos iteradores ordenados (de diferentes SSTables/memtables) em um único stream ordenado.
- **Column Family**: Namespace isolado de key-value dentro do banco (similar a "tabela").
