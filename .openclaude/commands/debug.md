# /debug — Diagnosticar Problema

Analisa um erro, panic ou comportamento inesperado e propõe solução.

## O que fazer

1. Leia `$ARGUMENTS` — pode ser:
   - Uma mensagem de erro colada
   - Um nome de arquivo/função suspeita
   - Um comportamento descrito em prosa
2. Consulte `.openclaude/error-catalog.md` para erros conhecidos
3. Se for um erro de compilação Rust:
   - Identifique o código de erro (`E0XXX`) e explique o que significa
   - Mostre o trecho problemático e a correção mínima
4. Se for um erro de runtime/panic:
   - Trace o caminho de execução pelo CLAUDE.md (fluxos de escrita/leitura)
   - Identifique qual camada (core/storage/infra/api) está envolvida
5. Proponha fix com `diff` quando possível

## Leia também

- `.openclaude/error-catalog.md`
- `.openclaude/skills/rust-lsm.md`
- `.openclaude/decisions.md`
