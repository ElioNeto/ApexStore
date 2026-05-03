# AGENTS.md

<!-- Este arquivo é gerado automaticamente pelo boilerplate-opencode. -->
<!-- Não edite manualmente a seção entre as tags AUTO-GENERATED. -->
<!-- Preencha as seções marcadas com > após a instalação. -->

## Projeto

> Nome do projeto, objetivo principal e contexto de negócio em 2-3 frases.

## Stack

> Liste as tecnologias principais: linguagem, framework, banco de dados, infra.
> Exemplo: Node.js 20 + TypeScript, Fastify, PostgreSQL, Docker.

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

> Preencha com os comandos exatos do projeto. O agente usará estes comandos diretamente.

```bash
# Instalar dependências
npm install

# Rodar testes
npm test

# Lint
npm run lint

# Build
npm run build

# Dev
npm run dev
```

## Convenções

> Preencha com as convenções do projeto.

- **Commits**: Conventional Commits (`feat`, `fix`, `chore`, `docs`, `refactor`)
- **Branches**: `feat/<slug>`, `fix/<slug>`, `chore/<slug>`
- **Naming**: snake_case para variáveis/funções, PascalCase para tipos/structs/enums
- **Testes**: arquivos `*.test.ts` ao lado do módulo testado
- **Estrutura de pastas**: descreva aqui

## Contexto de domínio

> Glossário de termos do negócio que o agente precisa entender para implementar corretamente.
> Exemplo: "Pedido" = entidade central; "Fulfillment" = processo de separação e envio.
