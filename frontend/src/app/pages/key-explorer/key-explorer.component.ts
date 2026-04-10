import { Component, inject, signal, computed } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { ApexStoreService, SearchResult } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Entry { key: string; value: string; fetchedAt: Date; }
type ViewMode = 'table' | 'scan';

@Component({
  selector: 'app-key-explorer',
  standalone: true,
  imports: [FormsModule, DatePipe],
  templateUrl: './key-explorer.component.html',
  styleUrl: './key-explorer.component.scss'
})
export class KeyExplorerComponent {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  lookupKey = '';
  insertKey = '';
  insertValue = '';
  searchQ = '';
  searchPrefix = false;
  batchJson = '';
  filterText = '';

  loadingLookup  = signal(false);
  loadingInsert  = signal(false);
  loadingSearch  = signal(false);
  loadingBatch   = signal(false);
  loadingList    = signal(false);
  loadingScan    = signal(false);
  entries        = signal<Entry[]>([]);
  viewMode       = signal<ViewMode>('table');

  filteredEntries = computed(() => {
    const q = this.filterText.toLowerCase();
    if (!q) return this.entries();
    return this.entries().filter(e => e.key.toLowerCase().includes(q) || e.value.toLowerCase().includes(q));
  });

  scanModeLabel = computed(() => {
    return this.viewMode() === 'scan' ? 'Scan Results' : 'Entries';
  });

  lookup(): void {
    const key = this.lookupKey.trim();
    if (!key) return;
    this.loadingLookup.set(true);
    this.store.get(key).subscribe({
      next: (r) => { this.upsert(r.key, r.value); this.toast.success(`Key "${key}" found!`); this.loadingLookup.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? `Key "${key}" not found`); this.loadingLookup.set(false); }
    });
  }

  insert(): void {
    const key = this.insertKey.trim(), value = this.insertValue.trim();
    if (!key || !value) return;
    this.loadingInsert.set(true);
    this.store.put(key, value).subscribe({
      next: () => { this.upsert(key, value); this.toast.success(`Key "${key}" inserted!`); this.insertKey = ''; this.insertValue = ''; this.loadingInsert.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Insert failed'); this.loadingInsert.set(false); }
    });
  }

  runSearch(): void {
    const q = this.searchQ.trim();
    if (!q) return;
    this.loadingSearch.set(true);
    this.store.search(q, this.searchPrefix).subscribe({
      next: (records) => {
        records.forEach(r => this.upsert(r.key, r.value));
        this.toast.success(`${records.length} keys found for "${q}"`);
        this.viewMode.set('table');
        this.loadingSearch.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Search failed'); this.loadingSearch.set(false); }
    });
  }

  runBatch(): void {
    let records: { key: string; value: string }[];
    try { records = JSON.parse(this.batchJson); } catch { this.toast.error('Invalid JSON'); return; }
    if (!Array.isArray(records) || records.length === 0) { this.toast.error('Expected a non-empty JSON array'); return; }
    this.loadingBatch.set(true);
    this.store.setBatch(records).subscribe({
      next: (r) => {
        records.forEach(rec => this.upsert(rec.key, rec.value));
        this.toast.success(r.message);
        this.batchJson = '';
        this.loadingBatch.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Batch insert failed'); this.loadingBatch.set(false); }
    });
  }

  loadAllKeys(): void {
    this.loadingList.set(true);
    this.store.listKeys().subscribe({
      next: (keys) => {
        keys.forEach(k => {
          if (!this.entries().find(e => e.key === k))
            this.entries.update(list => [...list, { key: k, value: '—', fetchedAt: new Date() }]);
        });
        this.toast.success(`${keys.length} keys listed`);
        this.viewMode.set('table');
        this.loadingList.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'List failed'); this.loadingList.set(false); }
    });
  }

  runScan(): void {
    this.loadingScan.set(true);
    this.store.scan().subscribe({
      next: (records) => {
        this.entries.set(records.map(r => ({ key: r.key, value: r.value, fetchedAt: new Date() })));
        this.toast.success(`${records.length} records loaded from scan`);
        this.viewMode.set('scan');
        this.loadingScan.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Scan failed'); this.loadingScan.set(false); }
    });
  }

  refetch(key: string): void {
    this.store.get(key).subscribe({
      next: (r) => { this.upsert(r.key, r.value); this.toast.info(`"${key}" refreshed`); },
      error: (e) => this.toast.error(e?.error?.message ?? 'Refetch failed')
    });
  }

  deleteKey(key: string): void {
    this.store.delete(key).subscribe({
      next: () => { this.entries.update(list => list.filter(e => e.key !== key)); this.toast.success(`Key "${key}" deleted`); },
      error: (e) => this.toast.error(e?.error?.message ?? 'Delete failed')
    });
  }

  clearEntries(): void { this.entries.set([]); }

  private upsert(key: string, value: string): void {
    this.entries.update(list => {
      const idx = list.findIndex(e => e.key === key);
      const entry = { key, value, fetchedAt: new Date() };
      if (idx >= 0) { const u = [...list]; u[idx] = entry; return u; }
      return [entry, ...list];
    });
  }
}
