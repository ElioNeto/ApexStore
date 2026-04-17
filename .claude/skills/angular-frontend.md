# Skill: Angular 17 Frontend (ApexStore)

Convenções e padrões do frontend Angular 17 do ApexStore. Carregue ao trabalhar em `frontend/`.

---

## Regras obrigatórias

### Componentes
- Todos **standalone** — sem NgModules
- Injeção com `inject()` — nunca no constructor
- Estado reativo exclusivamente com **Signals**: `signal()`, `computed()`, `input()`
- Template syntax nova: `@if`, `@for`, `@switch` — NUNCA `*ngIf`, `*ngFor`

```typescript
// ✅ CORRETO
@Component({ standalone: true, ... })
export class MyComponent {
  private svc = inject(ApexStoreService);
  items = signal<string[]>([]);
  count = computed(() => this.items().length);
}

// ❌ ERRADO
constructor(private svc: ApexStoreService) {}
@Input() value: string;  // use input() signal
```

### Serviços
```typescript
// Sempre providedIn: 'root'
@Injectable({ providedIn: 'root' })
export class ApexStoreService {
  private http = inject(HttpClient);
  // Retornar Observable — consumidor decide se converte para signal
}
```

### Templates
```html
<!-- ✅ CORRETO -->
@if (items().length > 0) {
  @for (item of items(); track item.key) {
    <app-stat-card [value]="item" />
  }
} @else {
  <p>Nenhum item encontrado.</p>
}

<!-- ❌ ERRADO -->
<div *ngIf="items.length > 0">...</div>
```

---

## Estrutura de páginas

```
frontend/src/app/
├── pages/
│   ├── dashboard/        # visão geral + stats em tempo real
│   ├── key-explorer/     # busca, scan, CRUD de chaves
│   └── stats/            # métricas detalhadas de performance
├── components/
│   ├── toast/            # ToastService + componente de notificação
│   └── stat-card/        # card reutilizável de métrica
└── services/
    ├── apex-store.service.ts   # HTTP client para a API
    └── toast.service.ts        # gerenciamento de notificações
```

---

## API base URL

Configurado em `frontend/src/environments/environment.ts`:
```typescript
export const environment = {
  production: false,
  apiUrl: 'http://localhost:8080'
};
```

---

## SCSS — variáveis globais

Definidas em `frontend/src/styles.scss`. Use sempre variáveis CSS:
```scss
// ✅
color: var(--color-primary);
padding: var(--space-4);

// ❌
color: #01696f;
padding: 16px;
```

---

## Checklist antes de commitar frontend

- [ ] Sem NgModules novos
- [ ] Sem `*ngIf` / `*ngFor` (usar `@if` / `@for`)
- [ ] Sem `constructor` para injeção (usar `inject()`)
- [ ] Signals para todo estado local
- [ ] `npm run build` sem erros
- [ ] `npm run lint` sem warnings
