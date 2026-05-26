import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  PlayIcon,
  DatabaseIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface SyncJob {
  id: string;
  name: string;
  source: string;
  target: string;
  mode: 'full' | 'incremental' | 'snapshot';
  status: 'running' | 'completed' | 'failed' | 'scheduled';
  records_synced: number;
  started_at: number;
  finished_at?: number;
}

@Component({
  selector: 'app-data-sync',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './data-sync.component.html',
  styleUrl: './data-sync.component.scss'
})
export class DataSyncComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly PlayIcon = PlayIcon;
  readonly DatabaseIcon = DatabaseIcon;

  jobs = signal<SyncJob[]>([]);
  loading = signal(false);
  creating = signal(false);

  newName = '';
  newSource = '';
  newTarget = '';
  newMode: 'full' | 'incremental' | 'snapshot' = 'incremental';

  modes: Array<{ value: string; label: string }> = [
    { value: 'full', label: 'Full Sync' },
    { value: 'incremental', label: 'Incremental' },
    { value: 'snapshot', label: 'Snapshot' },
  ];

  ngOnInit(): void { this.loadJobs(); }

  loadJobs(): void {
    this.loading.set(true);
    this.store.listSyncJobs().subscribe({
      next: (data) => {
        this.jobs.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load sync jobs');
        this.loading.set(false);
      }
    });
  }

  createJob(): void {
    if (!this.newName.trim() || !this.newSource.trim() || !this.newTarget.trim()) return;
    this.creating.set(true);
    this.store.createSyncJob(this.newName.trim(), this.newSource.trim(), this.newTarget.trim(), this.newMode).subscribe({
      next: () => {
        this.toast.success('Sync job created!');
        this.newName = '';
        this.creating.set(false);
        this.loadJobs();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create sync job');
        this.creating.set(false);
      }
    });
  }

  triggerSync(id: string): void {
    this.store.triggerSyncJob(id).subscribe({
      next: () => {
        this.toast.success('Sync triggered!');
        this.loadJobs();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to trigger sync')
    });
  }

  deleteJob(id: string): void {
    if (!confirm('Delete this sync job?')) return;
    this.store.deleteSyncJob(id).subscribe({
      next: () => {
        this.toast.success('Sync job deleted');
        this.jobs.update(list => list.filter(j => j.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete sync job')
    });
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
