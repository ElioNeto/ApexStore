# /bench — Rodar Benchmarks

Executa benchmarks com Criterion e interpreta os resultados.

## O que fazer

1. Rode `cargo bench 2>&1 | tee /tmp/bench-out.txt`
2. Extraia para cada benchmark:
   - Nome do bench
   - Tempo médio (ns/µs/ms)
   - Variação (lower/upper bound)
   - Comparação com baseline se disponível (`change: X%`)
3. Destaque regressões (piora > 5%) em 🔴 e melhorias (> 5%) em 🟢
4. Se `$ARGUMENTS` for fornecido: `cargo bench $ARGUMENTS`

## Benchmarks disponíveis

Localização: `benches/`
- `engine_bench` — throughput de put/get na LSM Engine
- Outros listados em `Cargo.toml` sob `[[bench]]`

## Leia também

- `.claude/skills/rust-lsm.md` — contexto de performance esperada
- `.claude/memory.md` — baseline de performance registrado
