import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  HardDriveIcon,
  DatabaseIcon,
  FileEditIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  SparklesIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface CompactionInfo {
  level: number;
  sst_count: number;
  size_kb: number;
  running: boolean;
  pending: boolean;
}

@Component({
  selector: 'app-compaction',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './compaction.component.html',
  styleUrl: './compaction.component.scss'
})
export class CompactionComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly HardDriveIcon = HardDriveIcon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly FileEditIcon = FileEditIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly SparklesIcon = SparklesIcon;

  levels = signal<CompactionInfo[]>([]);
  memRecords = signal(0);
  walKb = signal(0);
  loading = signal(false);
  flushing = signal(false);
  compacting = signal(false);
  lastRefresh = signal<Date | null>(null);

  ngOnInit(): void { this.loadStats(); }

  loadStats(): void {
    this.loading.set(true);
    this.store.getStats().subscribe({
      next: (data) => {
        this.parseStats(data);
        this.lastRefresh.set(new Date());
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load compaction status');
        this.loading.set(false);
      }
    });
  }

  private parseStats(data: Record<string, unknown>): void {
    this.memRecords.set(Number(data['mem_records'] ?? 0));
    this.walKb.set(Number(data['wal_kb'] ?? 0));

    const parsed: CompactionInfo[] = [];
    const sstable = data['sstable'] as Record<string, unknown> | undefined;
    if (sstable) {
      for (let i = 0; i <= 6; i++) {
        const key = `L${i}`;
        const level = sstable[key] as Record<string, unknown> | undefined;
        if (level) {
          parsed.push({
            level: i,
            sst_count: Number(level['files'] ?? level['sst_count'] ?? 0),
            size_kb: Number(level['size_kb'] ?? 0),
            running: level['compaction_running'] === true,
            pending: level['compaction_pending'] === true,
          });
        }
      }
    }
    this.levels.set(parsed);
  }

  flushMemtable(): void {
    this.flushing.set(true);
    this.store.flush().subscribe({
      next: () => {
        this.toast.success('Memtable flushed successfully!');
        this.flushing.set(false);
        this.loadStats();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Flush failed');
        this.flushing.set(false);
      }
    });
  }

  triggerCompaction(): void {
    this.compacting.set(true);
    this.store.compact().subscribe({
      next: () => {
        this.toast.success('Compaction triggered successfully!');
        this.compacting.set(false);
        this.loadStats();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Compaction failed');
        this.compacting.set(false);
      }
    });
  }

  formatSize(kb: number): string {
    if (kb > 1_048_576) return (kb / 1_048_576).toFixed(2) + ' GB';
    if (kb > 1_024) return (kb / 1_024).toFixed(2) + ' MB';
    return kb.toFixed(0) + ' KB';
  }
}
