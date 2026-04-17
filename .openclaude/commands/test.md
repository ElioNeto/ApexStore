# /test — Rodar e Analisar Testes

Executa a suite de testes do ApexStore e interpreta os resultados.

## O que fazer

1. Rode `cargo test 2>&1` e capture a saída completa
2. Separe em três grupos:
   - ✅ Passou
   - ❌ Falhou (mostre nome do teste + mensagem de erro)
   - ⚠️  Ignorado
3. Para cada falha, leia o código-fonte do teste em `src/` ou `tests/` e explique:
   - O que o teste estava verificando
   - Qual foi o comportamento real vs esperado
   - Sugestão de correção
4. Se `$ARGUMENTS` for fornecido, filtre os testes: `cargo test $ARGUMENTS`

## Exemplos de uso

```
/test                  → roda todos os testes
/test engine           → roda testes com "engine" no nome
/test storage::wal     → roda testes do módulo WAL
```

## Leia também

- `.openclaude/skills/rust-lsm.md` — contexto da engine
- `.openclaude/error-catalog.md` — erros conhecidos
