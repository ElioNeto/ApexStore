# Skill: Padrões Angular 17 — ApexStore Frontend

Use esta skill ao escrever qualquer código Angular no projeto (`frontend/`).

## Regras absolutas

| ❌ Proibido | ✅ Correto |
|---|---|
| `*ngIf` | `@if` |
| `*ngFor` | `@for ... track` |
| `NgModule` | `standalone: true` |
| `constructor(private svc: Service)` | `svc = inject(Service)` |
| `@Input() valor: string` | `valor = input<string>()` |
| `@Output() evento` | `evento = output<Tipo>()` |
| `this.valor` mutando diretamente | `this.valor.set(novoValor)` |

## Signals — guia rápido

```typescript
import { signal, computed, effect, input, output } from '@angular/core';

// Estado local
loading = signal(false);
items = signal<string[]>([]);

// Derivado (nunca duplicar estado)
count = computed(() => this.items().length);
empty = computed(() => this.items().length === 0);

// Input reativo (substitui @Input)
value = input<string>('');           // com default
requiredValue = input.required<number>();

// Output (substitui @Output + EventEmitter)
onSelect = output<string>();
// uso: this.onSelect.emit('valor');

// Atualização
this.loading.set(true);
this.items.update(list => [...list, novoItem]);
this.items.set([]);  // reset
```

## Template syntax

```html
<!-- Condicional -->
@if (loading()) {
  <span class="spinner"></span>
} @else if (error()) {
  <div class="error">{{ errorMsg() }}</div>
} @else {
  <div>conteúdo</div>
}

<!-- Lista — track é obrigatório -->
@for (item of items(); track item.id) {
  <div>{{ item.name }}</div>
} @empty {
  <div>Nenhum item encontrado.</div>
}

<!-- Switch -->
@switch (status()) {
  @case ('loading') { <span class="spinner"></span> }
  @case ('error') { <span>Erro</span> }
  @default { <span>OK</span> }
}
```

## Estrutura de componente

```typescript
import { Component, inject, signal, computed, input, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-meu-componente',
  standalone: true,
  imports: [FormsModule],   // só o necessário
  template: `...`,
  styles: [`...`]           // styles inline para componentes simples
})
export class MeuComponente implements OnInit {
  // 1. Injeções
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  // 2. Inputs
  title = input<string>('Default');

  // 3. Estado interno
  loading = signal(false);
  data = signal<string[]>([]);

  // 4. Derivados
  isEmpty = computed(() => this.data().length === 0);

  // 5. Lifecycle
  ngOnInit(): void {
    this.load();
  }

  // 6. Métodos — sempre tipados
  load(): void {
    this.loading.set(true);
    this.store.get('key').subscribe({
      next: (res) => {
        this.data.set([res.value]);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err?.error?.message ?? 'Erro desconhecido');
        this.loading.set(false);
      }
    });
  }
}
```

## HTTP e serviços

Todo acesso HTTP passa pelo `ApexStoreService`. Nunca injetar `HttpClient` diretamente em componentes.

```typescript
// ✅ No componente
this.store.get(key).subscribe({ next: ..., error: ... });

// ✅ No ApexStoreService
public meuEndpoint(param: string): Observable<MinhaResposta> {
  return this.http.get<MinhaResposta>(`${this.baseUrl}/endpoint/${param}`);
}
```

Sempre assine com `{ next, error }` — nunca ignore o error handler.

## Estilos

Use as CSS custom properties definidas em `styles.scss`:

```scss
// Cores
var(--bg-primary)      // fundo da página
var(--bg-secondary)    // fundo sidebar/headers
var(--bg-card)         // fundo de cards
var(--border)          // bordas
var(--accent)          // laranja — ação primária
var(--green)           // sucesso
var(--red)             // erro
var(--blue)            // info
var(--text-primary)    // texto principal
var(--text-secondary)  // texto secundário
var(--text-muted)      // texto desativado
var(--font-sans)       // Inter
var(--font-mono)       // JetBrains Mono
var(--radius)          // 10px
var(--radius-lg)       // 16px

// Classes utilitárias (já definidas em styles.scss)
.btn .btn-primary .btn-secondary .btn-danger .btn-success .btn-sm
.badge .badge-success .badge-danger .badge-info .badge-warning
.input-group
.spinner
```

## Adicionando nova página

1. Criar `frontend/src/app/pages/<nome>/<nome>.component.ts`
2. Adicionar em `app.routes.ts`:
```typescript
{ path: '<nome>', component: <Nome>Component }
```
3. Adicionar em `AppComponent.navItems` signal:
```typescript
{ path: '/<nome>', icon: '🔧', label: 'Nome' }
```
