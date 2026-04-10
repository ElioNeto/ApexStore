import { Component, inject, signal, OnInit, computed } from '@angular/core';
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
  templateUrl: './dashboard.component.html',
  styleUrl: './dashboard.component.scss'
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

  filteredHistory = computed(() => {
    const q = this.history().filter(h =>
      h.key.toLowerCase().includes(this.historyFilter.toLowerCase()) ||
      (h.result?.toLowerCase().includes(this.historyFilter.toLowerCase()) ?? false)
    );
    return q;
  });

  historyFilter = '';

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
    const memKb = typeof data['mem_kb'] === 'number' ? data['mem_kb'] : Number(data['mem_kb'] ?? 0);
    const memRecords = typeof data['mem_records'] === 'number' ? data['mem_records'] : Number(data['mem_records'] ?? 0);
    const memtableMaxSize = typeof data['memtable_max_size'] === 'number' ? data['memtable_max_size'] : Number(data['memtable_max_size'] ?? 0);
    const sstFiles = typeof data['sst_files'] === 'number' ? data['sst_files'] : Number(data['sst_files'] ?? 0);
    const sstKb = typeof data['sst_kb'] === 'number' ? data['sst_kb'] : Number(data['sst_kb'] ?? 0);
    const sstRecords = typeof data['sst_records'] === 'number' ? data['sst_records'] : Number(data['sst_records'] ?? 0);
    const totalRecords = typeof data['total_records'] === 'number' ? data['total_records'] : Number(data['total_records'] ?? 0);
    const walKb = typeof data['wal_kb'] === 'number' ? data['wal_kb'] : Number(data['wal_kb'] ?? 0);

    // Convert KB to MB for better display
    const memMemtableMb = (memKb / 1024).toFixed(2);
    const diskUsageMb = (sstKb / 1024).toFixed(2);

    const cards = [
      { icon: 'Memory', label: 'MemTable', value: `${memRecords} records`, sub: `${memMemtableMb} MB` },
      { icon: 'WAL', label: 'WAL', value: `${walKb} KB`, sub: 'Write-ahead log' },
      { icon: 'Disk', label: 'Disk Usage', value: `${diskUsageMb} MB`, sub: `${sstFiles} SSTables` },
      { icon: 'Keys', label: 'Total Keys', value: `${totalRecords}`, sub: 'All records' },
    ];

    this.statCards.set(cards);
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
