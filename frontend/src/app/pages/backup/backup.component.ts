import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  HardDriveIcon,
  Add01Icon,
  Delete01Icon,
  ArrowRight01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Backup {
  id: string;
  name: string;
  size_kb: number;
  created_at: number;
  status: 'completed' | 'running' | 'failed';
}

@Component({
  selector: 'app-backup',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './backup.component.html',
  styleUrl: './backup.component.scss'
})
export class BackupComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly HardDriveIcon = HardDriveIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly ArrowRight01Icon = ArrowRight01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;

  backups = signal<Backup[]>([]);
  loading = signal(false);
  creating = signal(false);
  backupName = '';

  ngOnInit(): void { this.loadBackups(); }

  loadBackups(): void {
    this.loading.set(true);
    this.store.listBackups().subscribe({
      next: (data) => {
        this.backups.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load backups');
        this.loading.set(false);
      }
    });
  }

  createBackup(): void {
    const name = this.backupName.trim() || `backup-${Date.now()}`;
    this.creating.set(true);
    this.store.createBackup(name).subscribe({
      next: () => {
        this.toast.success(`Backup "${name}" created successfully!`);
        this.backupName = '';
        this.creating.set(false);
        this.loadBackups();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Backup creation failed');
        this.creating.set(false);
      }
    });
  }

  restoreBackup(id: string, name: string): void {
    if (!confirm(`Restore backup "${name}"? This will overwrite current data.`)) return;
    this.store.restoreBackup(id).subscribe({
      next: () => {
        this.toast.success(`Backup "${name}" restored!`);
        this.loadBackups();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Restore failed')
    });
  }

  deleteBackup(id: string, name: string): void {
    if (!confirm(`Delete backup "${name}"?`)) return;
    this.store.deleteBackup(id).subscribe({
      next: () => {
        this.toast.success(`Backup "${name}" deleted`);
        this.backups.update(list => list.filter(b => b.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Delete failed')
    });
  }

  formatSize(kb: number): string {
    if (kb > 1_048_576) return (kb / 1_048_576).toFixed(2) + ' GB';
    if (kb > 1_024) return (kb / 1_024).toFixed(2) + ' MB';
    return kb.toFixed(0) + ' KB';
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
