import { Component, inject, signal, OnInit } from '@angular/core';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';
import { JsonPipe, KeyValuePipe } from '@angular/common';

interface StatSection {
  label: string;
  icon: string;
  color: string;
  data: Record<string, unknown>;
}

@Component({
  selector: 'app-stats',
  standalone: true,
  imports: [JsonPipe, KeyValuePipe],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">📊 Statistics</h1>
          <p class="page-subtitle">Real-time telemetry from the storage engine</p>
        </div>
        <button class="btn btn-primary" (click)="loadStats()" [disabled]="loading()">
          @if (loading()) { <span class="spinner"></span> } @else { 🔄 }
          Refresh
        </button>
      </div>

      @if (loading() && sections().length === 0) {
        <div class="loading-state">
          <span class="spinner" style="width:32px;height:32px;border-width:3px"></span>
          <p>Loading statistics...</p>
        </div>
      }

      @if (error()) {
        <div class="error-banner">
          <span>⚠️</span>
          <div>
            <strong>Could not connect to ApexStore API</strong>
            <p>Make sure the backend is running on <code>{{ apiUrl }}</code></p>
          </div>
          <button class="btn btn-secondary btn-sm" (click)="loadStats()">Retry</button>
        </div>
      }

      @if (sections().length > 0) {
        <div class="sections-grid">
          @for (section of sections(); track section.label) {
            <div class="section-card" [style.--accent-color]="section.color">
              <div class="section-header">
                <span class="section-icon">{{ section.icon }}</span>
                <span class="section-label">{{ section.label }}</span>
              </div>
              <div class="section-body">
                @for (item of section.data | keyvalue; track item.key) {
                  <div class="stat-row">
                    <span class="stat-key">{{ item.key }}</span>
                    <span class="stat-val">{{ formatValue(item.value) }}</span>
                  </div>
                }
              </div>
            </div>
          }
        </div>

        <div class="card raw-card">
          <div class="card-header">
            <span class="card-title">🧾 Raw JSON Response</span>
          </div>
          <pre class="raw-json">{{ rawData() | json }}</pre>
        </div>
      }
    </div>
  `,
  styles: [`
    .page { padding: 32px; max-width: 1100px; }
    .page-header { display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 28px; flex-wrap: wrap; gap: 16px; }
    .page-title { font-size: 1.6rem; font-weight: 700; }
    .page-subtitle { color: var(--text-muted); font-size: 0.9rem; margin-top: 4px; }

    .loading-state { display: flex; flex-direction: column; align-items: center; gap: 16px; padding: 80px; color: var(--text-muted); }

    .error-banner {
      display: flex;
      align-items: flex-start;
      gap: 14px;
      padding: 20px;
      background: var(--red-dim);
      border: 1px solid rgba(239,68,68,0.3);
      border-radius: var(--radius);
      margin-bottom: 24px;
      font-size: 0.9rem;

      strong { display: block; color: var(--red); margin-bottom: 4px; }
      p { color: var(--text-secondary); }
      code { font-family: var(--font-mono); background: rgba(0,0,0,0.3); padding: 2px 6px; border-radius: 4px; }
    }

    .sections-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 20px; margin-bottom: 24px; }

    .section-card {
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: var(--radius-lg);
      overflow: hidden;
      border-top: 3px solid var(--accent-color, var(--accent));
    }
    .section-header { display: flex; align-items: center; gap: 10px; padding: 14px 18px; background: var(--bg-secondary); }
    .section-icon { font-size: 1.2rem; }
    .section-label { font-weight: 600; font-size: 0.9rem; color: var(--text-primary); }
    .section-body { padding: 4px 0; }

    .stat-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 10px 18px;
      border-bottom: 1px solid var(--border);
      &:last-child { border-bottom: none; }
    }
    .stat-key { font-size: 0.82rem; color: var(--text-secondary); font-family: var(--font-mono); }
    .stat-val { font-size: 0.85rem; font-family: var(--font-mono); color: var(--text-primary); font-weight: 500; }

    .card { background: var(--bg-card); border: 1px solid var(--border); border-radius: var(--radius-lg); overflow: hidden; }
    .card-header { display: flex; align-items: center; gap: 12px; padding: 16px 20px; border-bottom: 1px solid var(--border); background: var(--bg-secondary); }
    .card-title { font-weight: 600; font-size: 0.95rem; }

    .raw-json { padding: 20px; font-family: var(--font-mono); font-size: 0.8rem; color: var(--text-secondary); overflow-x: auto; line-height: 1.7; white-space: pre-wrap; word-break: break-all; }
  `]
})
export class StatsComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  loading = signal(false);
  error = signal(false);
  sections = signal<StatSection[]>([]);
  rawData = signal<unknown>(null);
  apiUrl = 'http://localhost:8080';

  private readonly sectionMap: Record<string, { label: string; icon: string; color: string }> = {
    memory: { label: 'Memory', icon: '🧠', color: '#3b82f6' },
    wal: { label: 'Write-Ahead Log', icon: '📝', color: '#f97316' },
    disk: { label: 'Disk', icon: '💽', color: '#a855f7' },
    bloom: { label: 'Bloom Filter', icon: '🌸', color: '#22c55e' },
    cache: { label: 'Block Cache', icon: '⚡', color: '#f59e0b' },
    sstable: { label: 'SSTable', icon: '📦', color: '#06b6d4' },
  };

  ngOnInit(): void {
    this.loadStats();
  }

  loadStats(): void {
    this.loading.set(true);
    this.error.set(false);
    this.store.getStats().subscribe({
      next: (data) => {
        this.rawData.set(data);
        this.parseSections(data as Record<string, unknown>);
        this.loading.set(false);
      },
      error: () => {
        this.error.set(true);
        this.loading.set(false);
        this.toast.error('Failed to load statistics. Is the API running?');
      }
    });
  }

  private parseSections(data: Record<string, unknown>): void {
    const result: StatSection[] = [];

    for (const key of Object.keys(data)) {
      const val = data[key];
      if (val && typeof val === 'object' && !Array.isArray(val)) {
        const meta = this.sectionMap[key] ?? { label: key, icon: '📋', color: 'var(--accent)' };
        result.push({ ...meta, data: val as Record<string, unknown> });
      }
    }

    if (result.length === 0) {
      result.push({
        label: 'All Stats',
        icon: '📊',
        color: 'var(--accent)',
        data: data,
      });
    }

    this.sections.set(result);
  }

  formatValue(val: unknown): string {
    if (val === null || val === undefined) return '—';
    if (typeof val === 'number') {
      if (val > 1_000_000) return (val / 1_000_000).toFixed(2) + 'M';
      if (val > 1_000) return (val / 1_000).toFixed(2) + 'K';
      return String(val);
    }
    if (typeof val === 'boolean') return val ? '✅ true' : '❌ false';
    return String(val);
  }
}
