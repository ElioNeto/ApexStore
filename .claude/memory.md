# Memory — ApexStore

Fatos persistentes sobre o projeto que o Claude deve sempre lembrar,
independente do contexto da conversa.

## Dono e contato
- **Autor**: Elio Neto (`netoo.elio@hotmail.com`, GitHub: `ElioNeto`)
- **Demo**: https://lsm-admin-dev.up.railway.app/
- **Docs**: https://elioneto.github.io/ApexStore/

## Versão atual
- Backend: `2.1.11` (campo `version` em `Cargo.toml`)
- A versão é auto-incrementada pelo CI no merge — nunca editar manualmente

## Decisões que já foram tomadas (não questionar)
- Serialização em disco: **bincode** (não mudar para JSON ou MessagePack)
- Concorrência: **parking_lot** (não `std::sync`)
- Compressor: **LZ4** via `lz4_flex` (não Snappy, não Zstd)
- Framework HTTP: **Actix-Web 4** (não Axum, não Warp)
- Frontend: **Angular 17 standalone** (não React, não Vue)
- ORM/DB externo: **nenhum** — ApexStore é o próprio storage engine
- Compaction: **não implementado ainda** (previsto para v3.0)

## Estrutura de branches
- `main` — branch principal, CI protegido
- Branches de feature: `feat/<descricao>`
- Hotfixes: `fix/<descricao>`
- Nunca commitar diretamente em `main` (exceto chore/docs pequenos)

## Portas e serviços
- API REST: `http://localhost:8080`
- Frontend Angular: `http://localhost:4200`
- Docker (compose): porta 8080 mapeada

## Dados de runtime
- Diretório padrão: `./data/`
- Arquivos WAL: `*.log`
- Arquivos SSTable: `*.sst`
- Nunca commitar `data/` (está no `.gitignore`)

## Itens do Roadmap ativos (prioridade)
1. `v2.2` — Storage iterators para range queries (`src/storage/iterator.rs` já existe, precisa integrar ao Engine)
2. `v2.3` — Concurrent read optimization (múltiplos readers sem lock global)
3. `v3.0` — Leveled/Tiered Compaction

## Padrões que o autor prefere
- Código Rust: verbose e explícito > cleverness
- Sem macros complexas quando funções simples resolvem
- Commits seguem Conventional Commits: `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`
- PRs pequenos e focados (uma responsabilidade por PR)
- Sem dependências novas sem justificativa sólida
