# 🔄 GitHub Workflows Documentation

ApexStore utiliza workflows automatizados do GitHub Actions para gerenciar o ciclo de vida do desenvolvimento, desde features até releases.

## 📊 Visão Geral

```
feature/fix branches → develop → release/vX.Y.Z → main
       │                │            │              │
       └── PR auto    └─ PR auto  └─ Issues*   └─ Tag + Release
          + comments*     criado       fechadas*
          em issues*                   

* = Opcional, apenas quando issues são referenciadas
```

## 🛠️ Workflows Disponíveis

### 1. Feature/Fix Workflow

**Arquivo**: `.github/workflows/feature-fix-workflow.yml`

**Trigger**: Push em branches `feature/**` ou `fix/**`

#### O que faz:

1. **Build & Test**
   - Compila o projeto: `cargo build --release --all-features`
   - Executa testes: `cargo test --all-features`
   - Verifica lint: `cargo clippy --all-features -- -D warnings`

2. **Cria PR para develop**
   - Detecta automaticamente se há commits novos
   - Cria PR para `develop` com:
     - Lista de issues referenciadas *(se houver)*
     - Resumo dos commits
     - Status dos testes
   - Não duplica PRs existentes

3. **Comenta em Issues** *(opcional)*
   - **Só roda se houver issues referenciadas**
   - Identifica issues mencionadas nos commits
   - Adiciona comentários **empilhados** com updates:
     - Commits recentes
     - Link para branch
     - Status do desenvolvimento

#### Como usar:

**Com issues:**
```bash
git checkout -b feature/minha-feature
git commit -m "feat: implement X (#123)"
git commit -m "fix: resolve Y (fixes #124)"
git push origin feature/minha-feature
```

**Sem issues (também funciona!):**
```bash
git checkout -b feature/refactoring
git commit -m "refactor: improve code structure"
git commit -m "chore: update dependencies"
git push origin feature/refactoring
# ✅ PR criado normalmente, sem seção de issues
```

---

### 2. Develop to Release Workflow

**Arquivo**: `.github/workflows/develop-to-release.yml`

**Triggers**:
- Push em `develop` → Cria/atualiza PR de release
- PR merged em `main` → Fecha issues automaticamente *(se houver)*

#### O que faz:

**No push para develop:**

1. **Determina versão**
   - Analisa commits desde última tag
   - Calcula bump (major/minor/patch):
     - `BREAKING CHANGE:`, `feat!:`, `fix!:` → Major
     - `feat:` → Minor
     - Outros → Patch

2. **Cria branch de release**
   - `release/vX.Y.Z`
   - Sincroniza com `develop`

3. **Cria PR para main**
   - Title: `🚀 Release vX.Y.Z`
   - Draft mode (requer aprovação)
   - Contém:
     - Tipo de release configurável (alpha/beta/lts)
     - Lista de issues resolvidas *(se houver)*
     - Changelog completo
     - Checklist de validação

**No merge do PR de release:**

4. **Fecha Issues Automaticamente** *(opcional)*
   - **Só roda se houver issues referenciadas**
   - Extrai issues referenciadas nos commits
   - Adiciona comentário final:
     ```
     ✅ Resolved in Release vX.Y.Z
     
     This issue has been fixed and released.
     Release: [View Release](link)
     ```
   - Fecha issue com razão "completed"
   - **Se não houver issues**: workflow completa normalmente sem erros

---

## 🏷️ Referência de Issues (Opcional)

### Quando Usar Issues

✅ **Use quando:**
- Está resolvendo um bug reportado
- Está implementando uma feature solicitada
- Quer rastreabilidade automática
- Quer notificações automáticas

⚪ **Não precisa usar quando:**
- Refatoração interna
- Updates de dependências
- Melhorias de performance sem issue
- Documentação
- Chores e tarefas menores

### Sintaxe Suportada:

```bash
# Qualquer uma dessas formas é detectada:
git commit -m "feat: add feature (#123)"
git commit -m "fix: resolve bug (fixes #124)"
git commit -m "refactor: improve code (closes #125)"
git commit -m "docs: update (resolved #126)"
```

### Keywords Reconhecidas:

- `close`, `closes`, `closed`
- `fix`, `fixes`, `fixed`
- `resolve`, `resolves`, `resolved`
- Simples: `#123`

---

## 📋 Exemplos de Fluxo

### Exemplo 1: Com Issues

```bash
# Issue: #31 - Implement Bearer Token Authentication

git checkout -b feature/bearer-auth
git commit -m "feat: add auth module (#31)"
git commit -m "feat: add auth config (#31)"
git push origin feature/bearer-auth

# ✅ Workflow roda:
#    - Build + Tests passam
#    - PR criado: feature/bearer-auth → develop
#    - Comentário adicionado à #31
#    - Issue listada no PR
```

### Exemplo 2: Sem Issues

```bash
# Refatoração geral - sem issue específica

git checkout -b refactor/improve-performance
git commit -m "refactor: optimize database queries"
git commit -m "perf: add caching layer"
git push origin refactor/improve-performance

# ✅ Workflow roda:
#    - Build + Tests passam
#    - PR criado: refactor/improve-performance → develop
#    - Sem seção de issues (normal!)
#    - Changelog mostra commits normalmente
```

### Exemplo 3: Release com Mix

```bash
# Merge para develop (alguns commits com issues, outros sem)

git checkout develop
git merge feature/bearer-auth  # tem issue #31
git merge refactor/performance  # sem issue
git push origin develop

# ✅ Workflow roda:
#    - Calcula versão: v2.1.0 → v2.2.0
#    - Cria branch: release/v2.2.0
#    - Cria PR: release/v2.2.0 → main
#    - Lista apenas issue #31 (que foi referenciada)
#    - Changelog mostra TODOS os commits

# Ao mergear PR de release:
# ✅ Issue #31 fechada automaticamente
# ✅ Commits sem issue ignorados (sem erro)
```

---

## 🔍 Comportamento dos Workflows

### Feature/Fix Workflow

| Situação | Comportamento |
|----------|---------------|
| Commits com issues | PR criado + issues listadas + comentários nas issues |
| Commits sem issues | PR criado + "No issues referenced" |
| Mix | PR criado + apenas issues encontradas listadas |
| Issues inexistentes | Ignora e continua (sem erro) |
| Issues já fechadas | Não comenta (skip silencioso) |

### Develop to Release Workflow

| Situação | Comportamento |
|----------|---------------|
| Commits com issues | PR lista issues + ao mergear fecha automaticamente |
| Commits sem issues | PR sem seção de issues + ao mergear completa normalmente |
| Mix | PR lista apenas issues encontradas |
| Issues inexistentes | Ignora e continua (log warning) |
| Issues já fechadas | Tenta fechar mas ignora erro |

---

## ⚙️ Logs e Debugging

### Mensagens Normais (não são erros)

```
ℹ️ No issues referenced in commits - skipping
```
**Significado**: Nenhuma issue foi mencionada. Normal para commits sem rastreamento.

```
⏭️ Skipping issue #123 (state: CLOSED)
```
**Significado**: Issue já estava fechada. Workflow pula automaticamente.

```
⚠️ Issue #999 not found - skipping
```
**Significado**: Issue não existe. Pode ser typo no commit, workflow continua.

---

## 📚 Boas Práticas

### Quando Referenciar Issues

✅ **Recomendado**:
```bash
# Bug fixes
git commit -m "fix: resolve authentication bug (fixes #54)"

# Features solicitadas
git commit -m "feat: add JWT support (#31)"

# Melhorias específicas
git commit -m "perf: optimize query (closes #67)"
```

### Quando NÃO Referenciar

✅ **Também aceitável**:
```bash
# Refatorações internas
git commit -m "refactor: restructure auth module"

# Updates de dependências
git commit -m "chore: update dependencies"

# Documentação
git commit -m "docs: add API examples"

# Pequenos fixes
git commit -m "style: fix formatting"
```

---

## ⚠️ Troubleshooting

### "Workflow não comentou na issue"

**Possíveis causas**:
1. ✅ **Normal**: Issue não foi referenciada no commit
2. ✅ **Normal**: Issue já estava fechada
3. ⚠️ **Verifique**: Número da issue está correto?
4. ⚠️ **Verifique**: Sintaxe de referência correta?

### "Issue não fechou após release"

**Possíveis causas**:
1. ✅ **Normal**: Issue não foi referenciada em nenhum commit
2. ✅ **Normal**: Issue já estava fechada
3. ⚠️ **Verifique**: PR foi mergeado (não apenas fechado)?
4. ⚠️ **Verifique**: Branch seguia padrão `release/*`?

### "Workflow falhou"

**Checklist**:
- [ ] Build passou localmente?
- [ ] Tests passaram?
- [ ] Clippy sem erros?
- [ ] Permissões do GitHub Actions habilitadas?

---

## 🎯 Resumo

### TL;DR

- ✅ **Issues são OPCIONAIS** - workflows funcionam com ou sem
- ✅ **Use issues para rastreabilidade** - fechamento automático é bonus
- ✅ **Sem issues é válido** - para refatorações, chores, etc
- ✅ **Mix é aceito** - alguns commits com, outros sem issues
- ✅ **Workflows são resilientes** - não quebram por falta de issues

### Fluxo Mínimo (sem issues)

```bash
1. feature/x → develop
   ✅ Build + Test + PR criado

2. develop → release/vX.Y.Z → main
   ✅ Versão + Tag + Changelog

Nenhuma issue necessária!
```

### Fluxo Completo (com issues)

```bash
1. feature/x (#123) → develop
   ✅ Build + Test + PR + Issue comentada

2. develop → release/vX.Y.Z → main
   ✅ Versão + Tag + Changelog + Issue #123 fechada

Rastreabilidade automática!
```

---

## 📚 Referências

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Workflow Syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Closing Issues via Commit Messages](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue)
- [GitHub CLI](https://cli.github.com/manual/)

---

**Mantenedores**: @ElioNeto

**Última atualização**: 2026-03-06
