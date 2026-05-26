import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe, JsonPipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Search01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  ZapIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface QueryResult {
  columns: string[];
  rows: Record<string, unknown>[];
  row_count: number;
  elapsed_ms: number;
}

@Component({
  selector: 'app-sql-runner',
  standalone: true,
  imports: [FormsModule, DatePipe, JsonPipe, HugeiconsIconComponent],
  templateUrl: './sql-runner.component.html',
  styleUrl: './sql-runner.component.scss'
})
export class SqlRunnerComponent {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Search01Icon = Search01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly ZapIcon = ZapIcon;

  query = '';
  result = signal<QueryResult | null>(null);
  error = signal<string | null>(null);
  loading = signal(false);
  history = signal<string[]>([]);

  executeQuery(): void {
    const q = this.query.trim();
    if (!q) return;

    this.loading.set(true);
    this.result.set(null);
    this.error.set(null);

    this.store.executeQuery(q).subscribe({
      next: (data) => {
        this.result.set(data);
        this.addToHistory(q);
        this.loading.set(false);
        this.toast.success(`Query returned ${data.row_count} rows in ${data.elapsed_ms}ms`);
      },
      error: (e) => {
        const msg = e?.error?.message ?? e?.message ?? 'Query execution failed';
        this.error.set(msg);
        this.loading.set(false);
      }
    });
  }

  private addToHistory(q: string): void {
    this.history.update(h => [q, ...h.filter(item => item !== q)].slice(0, 20));
  }

  loadFromHistory(q: string): void {
    this.query = q;
    this.executeQuery();
  }

  clearHistory(): void {
    this.history.set([]);
  }

  clearResult(): void {
    this.result.set(null);
    this.error.set(null);
  }
}
