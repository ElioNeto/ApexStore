import { Component, inject, signal, OnInit, computed } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  ArrowRight01Icon,
  DatabaseIcon,
  FileEditIcon,
  HardDriveIcon,
  KeyIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';
import { StatCardComponent } from '../../components/stat-card/stat-card.component';
import type { IconSvgObject } from '@hugeicons/angular';

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
  imports: [FormsModule, DatePipe, StatCardComponent, HugeiconsIconComponent],
  templateUrl: './dashboard.component.html',
  styleUrl: './dashboard.component.scss'
})
export class DashboardComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  // HugeIcons references for template
  readonly RefreshIcon          = RefreshIcon;
  readonly ArrowRight01Icon     = ArrowRight01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon     = CancelCircleIcon;

  putKey = '';
  putValue = '';
  getKey = '';

  loadingPut   = signal(false);
  loadingGet   = signal(false);
  loadingStats = signal(false);
  getResult    = signal<string | null>(null);
  getError     = signal(false);
  history      = signal<HistoryEntry[]>([]);
  statCards    = signal<Array<{ icon: IconSvgObject; label: string; value: string; sub: string }>>([]);

  filteredHistory = computed(() =>
    this.history().filter(h =>
      h.key.toLowerCase().includes(this.historyFilter.toLowerCase()) ||
      (h.result?.toLowerCase().includes(this.historyFilter.toLowerCase()) ?? false)
    )
  );

  historyFilter = '';

  ngOnInit(): void { this.loadStats(); }

  loadStats(): void {
    this.loadingStats.set(true);
    this.store.getStats().subscribe({
      next: (data) => { this.buildStatCards(data); this.loadingStats.set(false); },
      error: ()    => { this.loadingStats.set(false); }
    });
  }

  private buildStatCards(data: Record<string, unknown>): void {
    const n = (k: string) => Number(data[k] ?? 0);
    const memMemtableMb = (n('mem_kb') / 1024).toFixed(2);
    const diskUsageMb   = (n('sst_kb') / 1024).toFixed(2);

    this.statCards.set([
      { icon: DatabaseIcon,    label: 'MemTable',    value: `${n('mem_records')} records`, sub: `${memMemtableMb} MB` },
      { icon: FileEditIcon,    label: 'WAL',         value: `${n('wal_kb')} KB`,           sub: 'Write-ahead log' },
      { icon: HardDriveIcon,   label: 'Disk Usage',  value: `${diskUsageMb} MB`,           sub: `${n('sst_files')} SSTables` },
      { icon: KeyIcon,         label: 'Total Keys',  value: `${n('total_records')}`,        sub: 'All records' },
    ]);
  }

  executePut(): void {
    if (!this.putKey.trim() || !this.putValue.trim()) return;
    this.loadingPut.set(true);
    this.store.put(this.putKey.trim(), this.putValue.trim()).subscribe({
      next: () => {
        this.toast.success(`Key "${this.putKey}" written successfully!`);
        this.addHistory('PUT', this.putKey, this.putValue, undefined, 'success');
        this.putKey = ''; this.putValue = '';
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
    this.getResult.set(null); this.getError.set(false);
    this.store.get(this.getKey.trim()).subscribe({
      next: (res) => {
        this.getResult.set(res.value); this.getError.set(false);
        this.toast.success('Key found!');
        this.addHistory('GET', this.getKey, undefined, res.value, 'success');
        this.loadingGet.set(false);
      },
      error: (err) => {
        const msg = err?.error?.message ?? 'Key not found';
        this.getResult.set(msg); this.getError.set(true);
        this.toast.error(msg);
        this.addHistory('GET', this.getKey, undefined, msg, 'error');
        this.loadingGet.set(false);
      }
    });
  }

  clearHistory(): void { this.history.set([]); }

  private addHistory(op: 'GET' | 'PUT', key: string, value?: string, result?: string, status: 'success' | 'error' = 'success'): void {
    this.history.update(h => [{ op, key, value, result, status, time: new Date() }, ...h].slice(0, 50));
  }
}
