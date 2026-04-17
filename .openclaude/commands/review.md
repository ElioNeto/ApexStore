# /review — Code Review de Diff

Faz review do diff atual ou de um PR específico.

## O que fazer

1. Se `$ARGUMENTS` for um número, revise o PR `#$ARGUMENTS` via `gh pr diff $ARGUMENTS`
2. Caso contrário, rode `git diff main...HEAD` para o diff local
3. Analise seguindo estas dimensões (ordene por severidade):

### 🔴 Bloqueadores
- `.unwrap()` / `.expect()` em código de produção (não em testes)
- Locks de leitura onde deveria haver escrow de escrita
- Paths de arquivo hardcoded
- Segredos ou credenciais no código

### 🟡 Melhorias
- Funções com mais de 50 linhas sem justificativa
- Ausência de testes para lógica nova
- Uso de `println!` em vez de `tracing::`
- Clone desnecessário de `String`/`Vec`

### 🟢 Sugestões
- Oportunidades de simplificação
- Nomes que poderiam ser mais descritivos
- Docstrings ausentes em funções públicas

4. Exiba as Issues em tabela: `| Arquivo:linha | Severidade | Descrição | Sugestão |`

## Leia também

- `.openclaude/skills/rust-lsm.md` — padrões do projeto
- `.openclaude/decisions.md` — o que NÃO mudar
