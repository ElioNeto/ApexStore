import { Component, inject, signal, OnInit } from '@angular/core';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';
import { JsonPipe, KeyValuePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  BrainIcon,
  FileXIcon,
  HardDriveIcon,
  SparklesIcon,
  CpuIcon,
  BotIcon,
  LeftTriangleIcon,
  RefreshIcon
} from '@hugeicons/core-free-icons';

interface StatSection {
  label: string;
  icon: typeof RefreshIcon;
  color: string;
  data: Record<string, unknown>;
}

@Component({
  selector: 'app-stats',
  standalone: true,
  imports: [JsonPipe, KeyValuePipe, HugeiconsIconComponent],
  templateUrl: './stats.component.html',
  styleUrl: './stats.component.scss'
})
export class StatsComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  loading = signal(false);
  error = signal(false);
  sections = signal<StatSection[]>([]);
  rawData = signal<unknown>(null);
  apiUrl = 'http://localhost:8080';

  readonly RefreshIcon = RefreshIcon;
  readonly LeftTriangleIcon = LeftTriangleIcon;

  readonly iconMap: Record<string, typeof RefreshIcon> = {
    memory: BrainIcon,
    wal: FileXIcon,
    disk: HardDriveIcon,
    bloom: SparklesIcon,
    cache: CpuIcon,
    sstable: BotIcon,
  };

  private readonly sectionMap: Record<string, { label: string; color: string }> = {
    memory: { label: 'Memory', color: '#3b82f6' },
    wal: { label: 'Write-Ahead Log', color: '#f97316' },
    disk: { label: 'Disk', color: '#a855f7' },
    bloom: { label: 'Bloom Filter', color: '#22c55e' },
    cache: { label: 'Block Cache', color: '#f59e0b' },
    sstable: { label: 'SSTable', color: '#06b6d4' },
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
        const meta = this.sectionMap[key] ?? { label: key, color: 'var(--accent)' };
        const icon = this.iconMap[key] ?? RefreshIcon;
        result.push({ ...meta, icon, data: val as Record<string, unknown> });
      }
    }

    if (result.length === 0) {
      result.push({
        label: 'All Stats',
        icon: RefreshIcon,
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
    return String(val);
  }
}
