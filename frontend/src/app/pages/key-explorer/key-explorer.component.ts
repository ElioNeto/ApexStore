import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';
import { DatePipe } from '@angular/common';

interface KeyEntry {
  key: string;
  value: string;
  fetchedAt: Date;
}

@Component({
  selector: 'app-key-explorer',
  standalone: true,
  imports: [FormsModule, DatePipe],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">🔑 Key Explorer</h1>
          <p class="page-subtitle">Lookup, insert, and manage key-value pairs</p>
        </div>
      </div>

      <!-- Lookup Form -->
      <div class="card">
        <div class="card-header">
          <span class="card-title">Lookup Key</span>
        </div>
        <div class="card-body">
          <div class="lookup-row">
            <div class="input-group" style="flex:1">
              <label>Key</label>
              <input
                [(ngModel)]="lookupKey"
                placeholder="Enter a key to lookup..."
                (keydown.enter)="lookupSingle()"
              />
            </div>
            <button
              class="btn btn-primary"
              style="align-self: flex-end"
              [disabled]="!lookupKey.trim() || loading()"
              (click)="lookupSingle()"
            >
              @if (loading()) { <span class="spinner"></span> } @else { 🔍 }
              Lookup
            </button>
          </div>
        </div>
      </div>

      <!-- Insert Form -->
      <div class="card" style="margin-top: 20px">
        <div class="card-header">
          <span class="op-badge put">POST</span>
          <span class="card-title">Insert Key-Value</span>
        </div>
        <div class="card-body">
          <div class="insert-row">
            <div class="input-group" style="flex:1">
              <label>Key</label>
              <input [(ngModel)]="insertKey" placeholder="key" />
            </div>
            <div class="input-group" style="flex:2">
              <label>Value</label>
              <input [(ngModel)]="insertValue" placeholder="value" (keydown.enter)="insertKV()" />
            </div>
            <button
              class="btn btn-primary"
              style="align-self: flex-end"
              [disabled]="!insertKey.trim() || !insertValue.trim() || loadingInsert()"
              (click)="insertKV()"
            >
              @if (loadingInsert()) { <span class="spinner"></span> } @else { ✏️ }
              Insert
            </button>
          </div>
        </div>
      </div>

      <!-- Results Table -->
      <div class="card" style="margin-top: 24px">
        <div class="card-header" style="justify-content: space-between">
          <div style="display:flex;align-items:center;gap:10px">
            <span class="card-title">Fetched Keys</span>
            <span class="badge badge-info">{{ entries().length }}</span>
          </div>
          @if (entries().length > 0) {
            <button class="btn btn-secondary btn-sm" (click)="clearEntries()">Clear all</button>
          }
        </div>

        @if (entries().length === 0) {
          <div class="empty-state">No keys fetched yet. Use Lookup above.</div>
        } @else {
          <div class="table-wrapper">
            <table class="kv-table">
              <thead>
                <tr>
                  <th>Key</th>
                  <th>Value</th>
                  <th>Fetched At</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                @for (entry of entries(); track entry.key) {
                  <tr>
                    <td class="mono key-cell">{{ entry.key }}</td>
                    <td class="mono val-cell">{{ entry.value }}</td>
                    <td class="time-cell">{{ entry.fetchedAt | date:'HH:mm:ss' }}</td>
                    <td>
                      <button
                        class="btn btn-secondary btn-sm"
                        (click)="refetch(entry.key)"
                        title="Refetch"
                      >🔄</button>
                      <button
                        class="btn btn-danger btn-sm"
                        (click)="removeEntry(entry.key)"
                        style="margin-left:6px"
                        title="Remove from list"
                      >✕</button>
                    </td>
                  </tr>
                }
              </tbody>
            </table>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    .page { padding: 32px; max-width: 1000px; }
    .page-header { margin-bottom: 28px; }
    .page-title { font-size: 1.6rem; font-weight: 700; }
    .page-subtitle { color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; }

    .card { background: var(--bg-card); border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; }
    .card-header { display: flex; align-items: center; gap: 12px; padding: 16px 20px; border-bottom: 1px solid var(--border); background: var(--bg-secondary); }
    .card-title { font-weight: 600; font-size: 0.95rem; }
    .card-body { padding: 20px; }

    .lookup-row, .insert-row { display: flex; gap: 12px; flex-wrap: wrap; }

    .op-badge { display: inline-block; padding: 3px 8px; border-radius: 5px; font-size: 0.72rem; font-weight: 700; font-family: var(--font-mono); }
    .op-badge.put { background: var(--accent-dim); color: var(--accent); }

    .empty-state { padding: 48px; text-align: center; color: var(--text-muted); font-size: 0.9rem; }

    .table-wrapper { overflow-x: auto; }
    .kv-table { width: 100%; border-collapse: collapse; }
    .kv-table th { padding: 12px 16px; text-align: left; font-size: 0.78rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-muted); border-bottom: 1px solid var(--border); }
    .kv-table td { padding: 12px 16px; border-bottom: 1px solid var(--border); font-size: 0.875rem; }
    .kv-table tr:last-child td { border-bottom: none; }
    .kv-table tr:hover td { background: var(--bg-secondary); }
    .mono { font-family: var(--font-mono); }
    .key-cell { color: var(--accent); }
    .val-cell { color: var(--text-primary); max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .time-cell { color: var(--text-muted); font-family: var(--font-mono); font-size: 0.8rem; }
  `]
})
export class KeyExplorerComponent {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  lookupKey = '';
  insertKey = '';
  insertValue = '';

  loading = signal(false);
  loadingInsert = signal(false);
  entries = signal<KeyEntry[]>([]);

  lookupSingle(): void {
    if (!this.lookupKey.trim()) return;
    const key = this.lookupKey.trim();
    this.loading.set(true);
    this.store.get(key).subscribe({
      next: (res) => {
        this.upsertEntry(key, res.value);
        this.toast.success(`Key "${key}" found!`);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err?.error?.message ?? `Key "${key}" not found`);
        this.loading.set(false);
      }
    });
  }

  insertKV(): void {
    if (!this.insertKey.trim() || !this.insertValue.trim()) return;
    const key = this.insertKey.trim();
    const value = this.insertValue.trim();
    this.loadingInsert.set(true);
    this.store.put(key, value).subscribe({
      next: () => {
        this.upsertEntry(key, value);
        this.toast.success(`Key "${key}" inserted!`);
        this.insertKey = '';
        this.insertValue = '';
        this.loadingInsert.set(false);
      },
      error: (err) => {
        this.toast.error(err?.error?.message ?? 'Insert failed');
        this.loadingInsert.set(false);
      }
    });
  }

  refetch(key: string): void {
    this.store.get(key).subscribe({
      next: (res) => {
        this.upsertEntry(key, res.value);
        this.toast.info(`Key "${key}" refreshed.`);
      },
      error: (err) => this.toast.error(err?.error?.message ?? 'Refetch failed')
    });
  }

  removeEntry(key: string): void {
    this.entries.update(e => e.filter(x => x.key !== key));
  }

  clearEntries(): void {
    this.entries.set([]);
  }

  private upsertEntry(key: string, value: string): void {
    this.entries.update(list => {
      const existing = list.findIndex(e => e.key === key);
      const entry = { key, value, fetchedAt: new Date() };
      if (existing >= 0) {
        const updated = [...list];
        updated[existing] = entry;
        return updated;
      }
      return [entry, ...list];
    });
  }
}
