# Skill: Angular 17 Frontend (ApexStore)

Convenções do frontend Angular 17. Carregue ao trabalhar em `frontend/`.

---

## Regras obrigatórias

- Todos os componentes **standalone** — sem NgModules
- Injeção com `inject()` — nunca no constructor
- Estado com **Signals**: `signal()`, `computed()`, `input()`
- Templates: `@if`, `@for` — NUNCA `*ngIf`, `*ngFor`

```typescript
// ✅
@Component({ standalone: true })
export class MyComponent {
  private svc = inject(ApexStoreService);
  items = signal<string[]>([]);
}

// ❌
constructor(private svc: ApexStoreService) {}
```

---

## Estrutura

```
frontend/src/app/
├── pages/        # dashboard, key-explorer, stats
├── components/   # toast, stat-card
└── services/     # ApexStoreService, ToastService
```

---

## Checklist frontend

- [ ] Sem NgModules novos
- [ ] Sem `*ngIf`/`*ngFor`
- [ ] Sem constructor para injeção
- [ ] Signals para todo estado local
- [ ] `npm run build` sem erros
- [ ] `npm run lint` sem warnings
