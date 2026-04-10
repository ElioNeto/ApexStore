import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';
import { StatCardComponent } from '../../components/stat-card/stat-card.component';

interface HistoryEntry {
  op: 'GET' | 'PUT';
  key: string;
  value?: string;
  result?: string;
  status: 'success' | 'error';
  time: Date;
}

@Component({
  selector: 'app-dashboard',
  standalone: true,
  imports: [FormsModule, DatePipe, StatCardComponent],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Dashboard</h1>
          <p class="page-subtitle">Interact with the ApexStore LSM-Tree engine in real time</p>
        </div>
        <button class="btn btn-secondary" (click)="loadStats()">
          @if (loadingStats()) { <span class="spinner"></span> }
          🔄 Refresh Stats
        </button>
      </div>

      <!-- Stats Row -->
      <div class="stats-grid">
        @for (card of statCards(); track card.label) {
          <app-stat-card
            [icon]="card.icon"
            [label]="card.label"
            [value]="card.value"
            [sub]="card.sub"
          />
        }
      </div>

      <!-- Operations -->
      <div class="ops-grid">
        <!-- PUT Operation -->
        <div class="card">
          <div class="card-header">
            <span class="op-badge put">POST</span>
            <span class="card-title">Insert / Update Key</span>
          </div>
          <div class="card-body">
            <div class="input-group">
              <label>Key</label>
              <input [(ngModel)]="putKey" placeholder="e.g. user:1" />
            </div>
            <div class="input-group" style="margin-top:12px">
              <label>Value</label>
              <textarea [(ngModel)]="putValue" placeholder="e.g. John Doe"></textarea>
            </div>
            <button
              class="btn btn-primary"
              style="width:100%;margin-top:14px;justify-content:center"
              [disabled]="!putKey.trim() || !putValue.trim() || loadingPut()"
              (click)="executePut()"
            >
              @if (loadingPut()) { <span class="spinner"></span> } @else { ✏️ }
              Write to Store
            </button>
          </div>
        </div>

        <!-- GET Operation -->
        <div class="card">
          <div class="card-header">
            <span class="op-badge get">GET</span>
            <span class="card-title">Retrieve Key</span>
          </div>
          <div class="card-body">
            <div class="input-group">
              <label>Key</label>
              <input [(ngModel)]="getKey" placeholder="e.g. user:1" (keydown.enter)="executeGet()" />
            </div>

            <button
              class="btn btn-success"
              style="width:100%;margin-top:14px;justify-content:center"
              [disabled]="!getKey.trim() || loadingGet()"
              (click)="executeGet()"
            >
              @if (loadingGet()) { <span class="spinner"></span> } @else { 🔍 }
              Fetch Value
            </button>

            @if (getResult() !== null) {
              <div class="result-box" [class.result-error]="getError()">
                <div class="result-label">{{ getError() ? 'Error' : 'Value' }}</div>
                <div class="result-value">{{ getResult() }}</div>
              </div>
            }
          </div>
        </div>
      </div>

      <!-- History -->
      <div class="card" style="margin-top: 24px">
        <div class="card-header" style="justify-content:space-between">
          <div style="display:flex;align-items:center;gap:10px">
            <span class="card-title">🕓 Operation History</span>
            <span class="badge badge-info">{{ history().length }}</span>
          </div>
          @if (history().length > 0) {
            <button class="btn btn-secondary btn-sm" (click)="clearHistory()">Clear</button>
          }
        </div>
        <div class="card-body" style="padding:0">
          @if (history().length === 0) {
            <div class="empty-state">No operations yet. Use PUT or GET above.</div>
          } @else {
            <div class="history-list">
              @for (entry of history(); track entry.time.getTime()) {
                <div class="history-item" [class.history-error]="entry.status === 'error'">
                  <span class="op-badge" [class.put]="entry.op === 'PUT'" [class.get]="entry.op === 'GET'">
                    {{ entry.op === 'PUT' ? 'POST' : 'GET' }}
                  </span>
                  <span class="history-key">{{ entry.key }}</span>
                  @if (entry.result) {
                    <span class="history-arrow">→</span>
                    <span class="history-val">{{ entry.result }}</span>
                  }
                  <span class="history-time">{{ entry.time | date:'HH:mm:ss' }}</span>
                </div>
              }
            </div>
          }
        </div>
      </div>
    </div>
  `,
  styles: [`
    .page { padding: 32px; max-width: 1100px; }
    .page-header { display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 28px; gap: 16px; flex-wrap: wrap; }
    .page-title { font-size: 1.6rem; font-weight: 700; color: var(--text-primary); }
    .page-subtitle { color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; }

    .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 28px; }

    .ops-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 20px; }
    @media (max-width: 700px) { .ops-grid { grid-template-columns: 1fr; } }

    .card {
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: var(--radius-lg);
      overflow: hidden;
    }
    .card-header {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 16px 20px;
      border-bottom: 1px solid var(--border);
      background: var(--bg-secondary);
    }
    .card-title { font-weight: 600; font-size: 0.95rem; color: var(--text-primary); }
    .card-body { padding: 20px; }

    .op-badge {
      display: inline-block;
      padding: 3px 8px;
      border-radius: 5px;
      font-size: 0.72rem;
      font-weight: 700;
      font-family: var(--font-mono);
      letter-spacing: 0.04em;
      &.put { background: var(--accent-dim); color: var(--accent); }
      &.get { background: var(--green-dim); color: var(--green); }
    }

    .result-box {
      margin-top: 14px;
      background: var(--bg-primary);
      border: 1px solid var(--border);
      border-radius: 8px;
      padding: 14px;
      &.result-error { border-color: var(--red); background: var(--red-dim); }
    }
    .result-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; margin-bottom: 6px; }
    .result-value { font-family: var(--font-mono); color: var(--text-primary); word-break: break-all; }

    .empty-state { padding: 40px; text-align: center; color: var(--text-muted); font-size: 0.9rem; }

    .history-list { display: flex; flex-direction: column; }
    .history-item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 12px 20px;
      border-bottom: 1px solid var(--border);
      font-size: 0.875rem;
      &:last-child { border-bottom: none; }
      &.history-error { background: rgba(239,68,68,0.04); }
    }
    .history-key { font-family: var(--font-mono); color: var(--text-primary); font-weight: 500; }
    .history-arrow { color: var(--text-muted); }
    .history-val { font-family: var(--font-mono); color: var(--text-secondary); flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 200px; }
    .history-time { margin-left: auto; color: var(--text-muted); font-size: 0.78rem; font-family: var(--font-mono); white-space: nowrap; }
  `]
})
export class DashboardComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  putKey = '';
  putValue = '';
  getKey = '';

  loadingPut = signal(false);
  loadingGet = signal(false);
  loadingStats = signal(false);
  getResult = signal<string | null>(null);
  getError = signal(false);
  history = signal<HistoryEntry[]>([]);
  statCards = signal<Array<{ icon: string; label: string; value: string; sub: string }>>([]);

  ngOnInit(): void {
    this.loadStats();
  }

  loadStats(): void {
    this.loadingStats.set(true);
    this.store.getStats().subscribe({
      next: (data) => {
        this.buildStatCards(data);
        this.loadingStats.set(false);
      },
      error: () => {
        this.loadingStats.set(false);
      }
    });
  }

  private buildStatCards(data: Record<string, unknown>): void {
    const memory = (data['memory'] as Record<string, unknown>) ?? {};
    const wal = (data['wal'] as Record<string, unknown>) ?? {};
    const disk = (data['disk'] as Record<string, unknown>) ?? {};

    const cards = [
      { icon: '🧠', label: 'MemTable Size', value: String(memory['memtable_size_bytes'] ?? '—'), sub: 'bytes used' },
      { icon: '📝', label: 'WAL Entries', value: String(wal['entry_count'] ?? '—'), sub: 'log entries' },
      { icon: '💽', label: 'Disk Usage', value: String(disk['total_bytes'] ?? '—'), sub: 'bytes on disk' },
      { icon: '🔢', label: 'Total Keys', value: String(memory['key_count'] ?? '—'), sub: 'in memtable' },
    ];

    const hasData = Object.keys(data).length > 0 && Object.values(cards).some(c => c.value !== '—');
    if (hasData) this.statCards.set(cards);
  }

  executePut(): void {
    if (!this.putKey.trim() || !this.putValue.trim()) return;
    this.loadingPut.set(true);
    this.store.put(this.putKey.trim(), this.putValue.trim()).subscribe({
      next: () => {
        this.toast.success(`Key "${this.putKey}" written successfully!`);
        this.addHistory('PUT', this.putKey, this.putValue, undefined, 'success');
        this.putKey = '';
        this.putValue = '';
        this.loadingPut.set(false);
      },
      error: (err) => {
        const msg = err?.error?.message ?? 'Failed to write key';
        this.toast.error(msg);
        this.addHistory('PUT', this.putKey, this.putValue, msg, 'error');
        this.loadingPut.set(false);
      }
    });
  }

  executeGet(): void {
    if (!this.getKey.trim()) return;
    this.loadingGet.set(true);
    this.getResult.set(null);
    this.getError.set(false);
    this.store.get(this.getKey.trim()).subscribe({
      next: (res) => {
        this.getResult.set(res.value);
        this.getError.set(false);
        this.toast.success(`Key found!`);
        this.addHistory('GET', this.getKey, undefined, res.value, 'success');
        this.loadingGet.set(false);
      },
      error: (err) => {
        const msg = err?.error?.message ?? 'Key not found';
        this.getResult.set(msg);
        this.getError.set(true);
        this.toast.error(msg);
        this.addHistory('GET', this.getKey, undefined, msg, 'error');
        this.loadingGet.set(false);
      }
    });
  }

  clearHistory(): void {
    this.history.set([]);
  }

  private addHistory(
    op: 'GET' | 'PUT',
    key: string,
    value?: string,
    result?: string,
    status: 'success' | 'error' = 'success'
  ): void {
    this.history.update(h => [{ op, key, value, result, status, time: new Date() }, ...h].slice(0, 50));
  }
}
