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
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Key Explorer</h1>
          <p class="page-subtitle">Lookup, search, insert, delete and full scan</p>
        </div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          <button class="btn btn-secondary" (click)="loadAllKeys()" [disabled]="loadingList()">
            @if (loadingList()) { <span class="spinner"></span> }
            List All Keys
          </button>
          <button class="btn btn-secondary" (click)="runScan()" [disabled]="loadingScan()">
            @if (loadingScan()) { <span class="spinner"></span> }
            Full Scan
          </button>
        </div>
      </div>

      <!-- Actions row -->
      <div class="actions-grid">
        <!-- Lookup -->
        <div class="card">
          <div class="card-header"><span class="op-badge get">GET</span><span class="card-title">Lookup Key</span></div>
          <div class="card-body">
            <div class="row-inline">
              <div class="input-group" style="flex:1">
                <label>Key</label>
                <input [(ngModel)]="lookupKey" placeholder="user:1" (keydown.enter)="lookup()" />
              </div>
              <button class="btn btn-success" style="align-self:flex-end" [disabled]="!lookupKey.trim() || loadingLookup()" (click)="lookup()">
                @if (loadingLookup()) { <span class="spinner"></span> }
                Lookup
              </button>
            </div>
          </div>
        </div>

        <!-- Insert -->
        <div class="card">
          <div class="card-header"><span class="op-badge put">POST</span><span class="card-title">Insert Key</span></div>
          <div class="card-body">
            <div class="row-inline">
              <div class="input-group">
                <label>Key</label>
                <input [(ngModel)]="insertKey" placeholder="key" />
              </div>
              <div class="input-group" style="flex:2">
                <label>Value</label>
                <input [(ngModel)]="insertValue" placeholder="value" (keydown.enter)="insert()" />
              </div>
              <button class="btn btn-primary" style="align-self:flex-end" [disabled]="!insertKey.trim()||!insertValue.trim()||loadingInsert()" (click)="insert()">
                @if (loadingInsert()) { <span class="spinner"></span> }
                Insert
              </button>
            </div>
          </div>
        </div>

        <!-- Search -->
        <div class="card">
          <div class="card-header"><span class="op-badge get">GET</span><span class="card-title">Search</span></div>
          <div class="card-body">
            <div class="row-inline">
              <div class="input-group" style="flex:1">
                <label>Query</label>
                <input [(ngModel)]="searchQ" placeholder="user:" (keydown.enter)="runSearch()" />
              </div>
              <label class="toggle-label">
                <input type="checkbox" [(ngModel)]="searchPrefix" />
                <span>Prefix</span>
              </label>
              <button class="btn btn-secondary" style="align-self:flex-end" [disabled]="!searchQ.trim()||loadingSearch()" (click)="runSearch()">
                @if (loadingSearch()) { <span class="spinner"></span> }
                Search
              </button>
            </div>
          </div>
        </div>

        <!-- Batch -->
        <div class="card">
          <div class="card-header"><span class="op-badge put">POST</span><span class="card-title">Batch Insert</span></div>
          <div class="card-body">
            <div class="input-group">
              <label>JSON array — [{"key":"k","value":"v"}, ...]</label>
              <textarea [(ngModel)]="batchJson" placeholder='[{"key":"k1","value":"v1"},{"key":"k2","value":"v2"}]'></textarea>
            </div>
            <button class="btn btn-primary" style="margin-top:10px;width:100%;justify-content:center" [disabled]="!batchJson.trim()||loadingBatch()" (click)="runBatch()">
              @if (loadingBatch()) { <span class="spinner"></span> }
              Insert Batch
            </button>
          </div>
        </div>
      </div>

      <!-- Results table -->
      <div class="card" style="margin-top:24px">
        <div class="card-header" style="justify-content:space-between">
          <div style="display:flex;align-items:center;gap:10px">
            <span class="card-title">{{ viewMode() === 'scan' ? 'Scan Results' : 'Entries' }}</span>
            <span class="badge badge-info">{{ entries().length }}</span>
            @if (filteredEntries().length !== entries().length) {
              <span class="badge badge-warning">{{ filteredEntries().length }} filtered</span>
            }
          </div>
          <div style="display:flex;gap:8px;align-items:center">
            <div class="input-group" style="flex-direction:row;align-items:center;gap:8px;margin:0">
              <input [(ngModel)]="filterText" placeholder="Filter..." style="width:160px;padding:6px 10px" />
            </div>
            @if (entries().length > 0) {
              <button class="btn btn-secondary btn-sm" (click)="clearEntries()">Clear</button>
            }
          </div>
        </div>

        @if (entries().length === 0) {
          <div class="empty-state">Use Lookup, Search or Full Scan to load data.</div>
        } @else {
          <div class="table-wrapper">
            <table class="kv-table">
              <thead>
                <tr>
                  <th>Key</th>
                  <th>Value</th>
                  <th>Time</th>
                  <th>Actions</th>
                </tr>
              </thead>
              <tbody>
                @for (e of filteredEntries(); track e.key) {
                  <tr>
                    <td class="mono key-cell">{{ e.key }}</td>
                    <td class="mono val-cell">{{ e.value }}</td>
                    <td class="time-cell">{{ e.fetchedAt | date:'HH:mm:ss' }}</td>
                    <td class="actions-cell">
                      <button class="btn btn-secondary btn-sm" (click)="refetch(e.key)" title="Refetch">Refetch</button>
                      <button class="btn btn-danger btn-sm" (click)="deleteKey(e.key)" title="Delete from store">Delete</button>
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
    .page { padding: 32px; max-width: 1200px; }
    .page-header { display:flex; align-items:flex-start; justify-content:space-between; margin-bottom:28px; gap:16px; flex-wrap:wrap; }
    .page-title { font-size:1.6rem; font-weight:700; }
    .page-subtitle { color:var(--text-muted); font-size:0.9rem; margin-top:4px; }
    .actions-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(280px,1fr)); gap:16px; }
    .card { background:var(--bg-card); border:1px solid var(--border); border-radius:var(--radius-lg); overflow:hidden; }
    .card-header { display:flex; align-items:center; gap:12px; padding:14px 18px; border-bottom:1px solid var(--border); background:var(--bg-secondary); }
    .card-title { font-weight:600; font-size:0.9rem; }
    .card-body { padding:16px; }
    .row-inline { display:flex; gap:10px; align-items:flex-end; flex-wrap:wrap; }
    .op-badge { display:inline-block; padding:3px 8px; border-radius:5px; font-size:0.7rem; font-weight:700; font-family:var(--font-mono); }
    .op-badge.put { background:var(--accent-dim); color:var(--accent); }
    .op-badge.get { background:var(--green-dim); color:var(--green); }
    .toggle-label { display:flex; align-items:center; gap:6px; font-size:0.85rem; color:var(--text-secondary); align-self:flex-end; padding-bottom:10px; cursor:pointer; white-space:nowrap; }
    .empty-state { padding:48px; text-align:center; color:var(--text-muted); font-size:0.9rem; }
    .table-wrapper { overflow-x:auto; }
    .kv-table { width:100%; border-collapse:collapse; }
    .kv-table th { padding:10px 16px; text-align:left; font-size:0.75rem; text-transform:uppercase; letter-spacing:0.06em; color:var(--text-muted); border-bottom:1px solid var(--border); }
    .kv-table td { padding:10px 16px; border-bottom:1px solid var(--border); font-size:0.875rem; }
    .kv-table tr:last-child td { border-bottom:none; }
    .kv-table tr:hover td { background:var(--bg-secondary); }
    .mono { font-family:var(--font-mono); }
    .key-cell { color:var(--accent); max-width:220px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
    .val-cell { color:var(--text-primary); max-width:300px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
    .time-cell { color:var(--text-muted); font-family:var(--font-mono); font-size:0.78rem; white-space:nowrap; }
    .actions-cell { display:flex; gap:6px; }
  `]
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
