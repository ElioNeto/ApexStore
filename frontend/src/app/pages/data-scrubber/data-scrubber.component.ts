import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  Key01Icon,
  PlayIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface ScrubberJob {
  id: string;
  pattern: string;
  retention_days: number;
  status: 'running' | 'completed' | 'failed' | 'scheduled';
  records_scrubbed: number;
  started_at: number;
  finished_at?: number;
}

interface IdempotencyKey {
  key: string;
  created_at: number;
  expires_at: number;
  ttl_sec: number;
  used: boolean;
}

@Component({
  selector: 'app-data-scrubber',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './data-scrubber.component.html',
  styleUrl: './data-scrubber.component.scss'
})
export class DataScrubberComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly Key01Icon = Key01Icon;
  readonly PlayIcon = PlayIcon;

  scrubberJobs = signal<ScrubberJob[]>([]);
  idempotencyKeys = signal<IdempotencyKey[]>([]);
  loading = signal(false);
  creating = signal(false);
  activeTab = signal<'scrubber' | 'idempotency'>('scrubber');

  newPattern = '';
  newRetention = 90;

  ngOnInit(): void { this.loadScrubberJobs(); }

  loadScrubberJobs(): void {
    this.loading.set(true);
    this.store.listScrubberJobs().subscribe({
      next: (data) => {
        this.scrubberJobs.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load scrubber jobs');
        this.loading.set(false);
      }
    });
  }

  loadIdempotencyKeys(): void {
    this.loading.set(true);
    this.store.listIdempotencyKeys().subscribe({
      next: (data) => {
        this.idempotencyKeys.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load idempotency keys');
        this.loading.set(false);
      }
    });
  }

  createScrubberJob(): void {
    if (!this.newPattern.trim()) return;
    this.creating.set(true);
    this.store.createScrubberJob(this.newPattern.trim(), this.newRetention).subscribe({
      next: () => {
        this.toast.success('Scrubber job created!');
        this.newPattern = '';
        this.creating.set(false);
        this.loadScrubberJobs();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create scrubber job');
        this.creating.set(false);
      }
    });
  }

  deleteScrubberJob(id: string): void {
    if (!confirm('Delete this scrubber job?')) return;
    this.store.deleteScrubberJob(id).subscribe({
      next: () => {
        this.toast.success('Scrubber job deleted');
        this.scrubberJobs.update(list => list.filter(j => j.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete scrubber job')
    });
  }

  deleteIdempotencyKey(key: string): void {
    this.store.deleteIdempotencyKey(key).subscribe({
      next: () => {
        this.toast.success('Idempotency key deleted');
        this.idempotencyKeys.update(list => list.filter(k => k.key !== key));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete idempotency key')
    });
  }

  switchTab(tab: 'scrubber' | 'idempotency'): void {
    this.activeTab.set(tab);
    if (tab === 'scrubber') this.loadScrubberJobs();
    else this.loadIdempotencyKeys();
  }

  statusClass(status: string): string {
    switch (status) {
      case 'completed': return 'badge-success';
      case 'running': return 'badge-warning';
      case 'failed': return 'badge-danger';
      default: return 'badge-info';
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
