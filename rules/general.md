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
