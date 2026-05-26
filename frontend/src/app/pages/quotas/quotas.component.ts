import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Add01Icon,
  Delete01Icon,
  Settings01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  DatabaseIcon,
  Edit01Icon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Quota {
  id: string;
  tenant: string;
  description: string;
  max_requests_per_sec: number;
  max_reads_per_sec: number;
  max_writes_per_sec: number;
  max_storage_mb: number;
  max_budget_per_day: number;
  current_budget_used: number;
  active: boolean;
  created_at: number;
}

@Component({
  selector: 'app-quotas',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './quotas.component.html',
  styleUrl: './quotas.component.scss'
})
export class QuotasComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly Settings01Icon = Settings01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly Edit01Icon = Edit01Icon;

  quotas = signal<Quota[]>([]);
  loading = signal(false);
  saving = signal(false);
  editingId = signal<string | null>(null);

  form: Partial<Quota> = {
    tenant: '',
    description: '',
    max_requests_per_sec: 1000,
    max_reads_per_sec: 500,
    max_writes_per_sec: 200,
    max_storage_mb: 1024,
    max_budget_per_day: 10000,
    active: true,
  };

  ngOnInit(): void { this.loadQuotas(); }

  loadQuotas(): void {
    this.loading.set(true);
    this.store.listQuotas().subscribe({
      next: (data) => {
        this.quotas.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load quotas');
        this.loading.set(false);
      }
    });
  }

  saveQuota(): void {
    if (!this.form.tenant?.trim()) return;
    this.saving.set(true);
    this.store.createQuota(this.form as Quota).subscribe({
      next: () => {
        this.toast.success('Quota saved!');
        this.resetForm();
        this.saving.set(false);
        this.loadQuotas();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to save quota');
        this.saving.set(false);
      }
    });
  }

  editQuota(q: Quota): void {
    this.form = { ...q };
    this.editingId.set(q.id);
  }

  updateQuota(): void {
    if (!this.editingId() || !this.form.tenant?.trim()) return;
    this.saving.set(true);
    this.store.updateQuota(this.editingId()!, this.form).subscribe({
      next: () => {
        this.toast.success('Quota updated!');
        this.resetForm();
        this.saving.set(false);
        this.loadQuotas();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to update quota');
        this.saving.set(false);
      }
    });
  }

  deleteQuota(id: string, tenant: string): void {
    if (!confirm(`Delete quota for "${tenant}"?`)) return;
    this.store.deleteQuota(id).subscribe({
      next: () => {
        this.toast.success('Quota deleted');
        this.quotas.update(list => list.filter(q => q.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete quota')
    });
  }

  resetForm(): void {
    this.form = { tenant: '', description: '', max_requests_per_sec: 1000, max_reads_per_sec: 500, max_writes_per_sec: 200, max_storage_mb: 1024, max_budget_per_day: 10000, active: true };
    this.editingId.set(null);
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
