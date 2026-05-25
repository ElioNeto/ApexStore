import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Upload01Icon,
  Download01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  FileEditIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface ImportJob {
  id: string;
  filename: string;
  format: 'json' | 'csv' | 'ndjson';
  status: 'pending' | 'running' | 'completed' | 'failed';
  records_total: number;
  records_imported: number;
  errors: number;
  created_at: number;
  finished_at?: number;
}

@Component({
  selector: 'app-bulk-import',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './bulk-import.component.html',
  styleUrl: './bulk-import.component.scss'
})
export class BulkImportComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Upload01Icon = Upload01Icon;
  readonly Download01Icon = Download01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly FileEditIcon = FileEditIcon;

  jobs = signal<ImportJob[]>([]);
  loading = signal(false);
  importing = signal(false);

  newFormat: 'json' | 'csv' | 'ndjson' = 'json';
  formats: Array<{ value: string; label: string }> = [
    { value: 'json', label: 'JSON' },
    { value: 'csv', label: 'CSV' },
    { value: 'ndjson', label: 'NDJSON' },
  ];

  ngOnInit(): void { this.loadJobs(); }

  loadJobs(): void {
    this.loading.set(true);
    this.store.listImportJobs().subscribe({
      next: (data) => {
        this.jobs.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load import jobs');
        this.loading.set(false);
      }
    });
  }

  onFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    if (!input.files?.length) return;
    const file = input.files[0];
    this.importing.set(true);
    this.store.createImportJob(file.name, this.newFormat).subscribe({
      next: () => {
        this.toast.success(`Import job created for "${file.name}"`);
        this.importing.set(false);
        this.loadJobs();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Import failed');
        this.importing.set(false);
      }
    });
    input.value = '';
  }

  deleteJob(id: string): void {
    if (!confirm('Delete this import job?')) return;
    this.store.deleteImportJob(id).subscribe({
      next: () => {
        this.toast.success('Import job deleted');
        this.jobs.update(list => list.filter(j => j.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete import job')
    });
  }

  downloadExport(): void {
    this.store.exportData('json').subscribe({
      next: (blob) => {
        const url = window.URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `apexstore-export-${Date.now()}.json`;
        a.click();
        window.URL.revokeObjectURL(url);
        this.toast.success('Export downloaded');
      },
      error: () => this.toast.error('Export failed')
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
