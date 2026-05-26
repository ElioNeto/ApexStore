import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-graphql',
  standalone: true,
  imports: [FormsModule],
  templateUrl: './graphql.component.html',
  styleUrl: './graphql.component.scss'
})
export class GraphQLComponent {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  query = signal(`{\n  stats {\n    total_records\n    sst_files\n    mem_records\n  }\n}`);
  result = signal<string | null>(null);
  error = signal(false);
  loading = signal(false);

  execute(): void {
    if (!this.query().trim()) return;
    this.loading.set(true);
    this.result.set(null);
    this.error.set(false);
    this.store.executeGraphQL(this.query().trim()).subscribe({
      next: (res) => {
        this.result.set(JSON.stringify(res, null, 2));
        this.loading.set(false);
      },
      error: (err) => {
        const msg = err?.error?.message ?? err.message ?? 'GraphQL request failed';
        this.result.set(JSON.stringify({ error: msg }, null, 2));
        this.error.set(true);
        this.loading.set(false);
        this.toast.error(msg);
      }
    });
  }
}
