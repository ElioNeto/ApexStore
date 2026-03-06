# 🔄 GitHub Workflows Documentation

ApexStore utiliza workflows automatizados do GitHub Actions para gerenciar o ciclo de vida do desenvolvimento, desde features até releases.

## 📊 Visão Geral

```
feature/fix branches → develop → release/vX.Y.Z → main
       │                │            │              │
       └── PR auto    └─ PR auto  └─ Issues    └─ Tag + Release
          + comments      criado       fechadas
          em issues
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
   - Cria PR draft para `develop` com:
     - Lista de issues referenciadas
     - Resumo dos commits
     - Status dos testes
   - Não duplica PRs existentes

3. **Comenta em Issues**
   - Identifica issues mencionadas nos commits
   - Adiciona comentários **empilhados** com updates:
     - Commits recentes
     - Link para branch
     - Status do desenvolvimento

#### Como usar:

```bash
# Criar feature branch
git checkout -b feature/minha-feature

# Fazer commits referenciando issues
git commit -m "feat: implement X (#123)"
git commit -m "fix: resolve Y (fixes #124)"

# Push - workflow roda automaticamente
git push origin feature/minha-feature
```

#### Comentários em Issues:

Cada push adiciona um novo comentário à issue:

```markdown
🔄 Update from `feature/minha-feature`

New commits pushed:
- feat: implement X (abc123)
- test: add tests for X (def456)

Status: In development
Branch: feature/minha-feature
Latest commit: abc123def456...
```

---

### 2. Develop to Release Workflow

**Arquivo**: `.github/workflows/develop-to-release.yml`

**Triggers**:
- Push em `develop` → Cria/atualiza PR de release
- PR merged em `main` → Fecha issues automaticamente

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
     - Lista de issues resolvidas
     - Changelog completo
     - Checklist de validação

**No merge do PR de release:**

4. **Fecha Issues Automaticamente**
   - Extrai issues referenciadas nos commits
   - Adiciona comentário final:
     ```
     ✅ Resolved in Release vX.Y.Z
     
     This issue has been fixed and released.
     Release: [View Release](link)
     ```
   - Fecha issue com razão "completed"

#### Como usar:

```bash
# Merge features para develop
git checkout develop
git merge feature/minha-feature
git push origin develop

# Workflow cria automaticamente:
# 1. Branch release/vX.Y.Z
# 2. PR draft: release/vX.Y.Z → main
```

**Configurar tipo de release no PR:**

Edite o corpo do PR e altere:

```markdown
Release Type: [lts]  # Altere para alpha, beta ou lts
```

**Aprovar release:**

1. Revise changelog
2. Marque checklist
3. Mude de draft para ready
4. Merge o PR
5. Issues serão fechadas automaticamente!

---

## 🏷️ Referência de Issues nos Commits

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

### Boas Práticas:

✅ **Recomendado**:
```bash
git commit -m "feat: implement authentication (#31)"
git commit -m "fix: resolve clippy warnings (#54)"
```

❌ **Evitar**:
```bash
git commit -m "update"  # Sem referência
git commit -m "fix stuff"  # Não menciona issue
```

---

## 📋 Exemplo de Fluxo Completo

### 1. Desenvolver Feature

```bash
# Issue: #31 - Implement Bearer Token Authentication

git checkout -b feature/bearer-auth
git commit -m "feat: add auth module (#31)"
git commit -m "feat: add auth config (#31)"
git commit -m "feat: integrate auth middleware (#31)"
git push origin feature/bearer-auth

# ✅ Workflow roda:
#    - Build + Tests passam
#    - PR criado: feature/bearer-auth → develop
#    - Comentário adicionado à #31
```

### 2. Merge para Develop

```bash
# Revisar e aprovar PR no GitHub
# Merge: feature/bearer-auth → develop

git checkout develop
git pull

# ✅ Workflow roda:
#    - Calcula versão: v2.1.0 → v2.2.0 (minor bump)
#    - Cria branch: release/v2.2.0
#    - Cria PR draft: release/v2.2.0 → main
#    - Lista issue #31 no PR
```

### 3. Release

```bash
# No GitHub:
# 1. Abrir PR: release/v2.2.0 → main
# 2. Editar tipo: Release Type: [lts]
# 3. Revisar changelog
# 4. Marcar checklist
# 5. Mudar de draft para ready
# 6. Merge PR

# ✅ Workflow roda:
#    - Adiciona comentário à #31:
#      "✅ Resolved in Release v2.2.0"
#    - Fecha issue #31
#    - Tag v2.2.0 criada
```

---

## 🔐 Permissões Necessárias

Os workflows requerem as seguintes permissões:

```yaml
permissions:
  contents: write      # Criar branches/tags
  pull-requests: write # Criar/editar PRs
  issues: write        # Comentar e fechar issues
```

Essas permissões são concedidas automaticamente ao `GITHUB_TOKEN`.

---

## ⚠️ Troubleshooting

### Workflow não rodou

**Sintomas**: Push feito mas workflow não aparece em Actions

**Soluções**:
1. Verificar nome da branch (`feature/*` ou `fix/*`)
2. Verificar se Actions está habilitado no repositório
3. Verificar permissões do GITHUB_TOKEN

### Issue não foi comentada

**Causas possíveis**:
1. Issue já estava fechada
2. Número da issue incorreto
3. Sintaxe de referência não reconhecida

**Debug**:
```bash
# Ver issues extraídas no log do workflow
# Actions → Workflow run → "Extract and comment on issues"
```

### Issue não fechou após release

**Causas**:
1. Issue não foi referenciada nos commits do release
2. PR não foi mergeado (apenas fechado)
3. Branch de release não segue padrão `release/*`

**Verificação**:
```bash
# Checar commits no release
git log release/vX.Y.Z --grep="#123"

# Deve retornar commits que mencionam a issue
```

---

## 📊 Métricas e Monitoring

### Visualizar Execuções

1. GitHub → Actions tab
2. Selecione workflow
3. Veja histórico de execuções

### Notificações

- Falhas de workflow notificam o autor do commit
- Comentários em issues notificam assignees
- Fechamento de issues notifica participantes

---

## 🚀 Próximos Passos

### Melhorias Futuras

- [ ] Geração automática de release notes
- [ ] Deploy automático após release
- [ ] Notificações no Slack/Discord
- [ ] Benchmark automático em PRs
- [ ] Criação de GitHub Release
- [ ] Publicação em crates.io

---

## 📚 Referências

- [GitHub Actions Documentation](https://docs.github.com/en/actions)
- [Workflow Syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Closing Issues via Commit Messages](https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue)
- [GitHub CLI](https://cli.github.com/manual/)

---

**Mantenedores**: @ElioNeto

**Última atualização**: 2026-03-06
