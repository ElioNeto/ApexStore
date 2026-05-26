import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Settings01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  CpuIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface BackpressureConfig {
  enabled: boolean;
  max_queue_size: number;
  max_concurrency: number;
  backoff_initial_ms: number;
  backoff_max_ms: number;
  backoff_factor: number;
  retry_max_attempts: number;
  retry_on_timeout: boolean;
  retry_on_error: boolean;
}

@Component({
  selector: 'app-backpressure',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './backpressure.component.html',
  styleUrl: './backpressure.component.scss'
})
export class BackpressureComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Settings01Icon = Settings01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;

  config = signal<BackpressureConfig>({
    enabled: true,
    max_queue_size: 10000,
    max_concurrency: 100,
    backoff_initial_ms: 100,
    backoff_max_ms: 30000,
    backoff_factor: 2,
    retry_max_attempts: 3,
    retry_on_timeout: true,
    retry_on_error: false,
  });
  loading = signal(false);
  saving = signal(false);

  ngOnInit(): void { this.loadConfig(); }

  loadConfig(): void {
    this.loading.set(true);
    this.store.getBackpressureConfig().subscribe({
      next: (data) => {
        this.config.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load backpressure configuration');
        this.loading.set(false);
      }
    });
  }

  saveConfig(): void {
    this.saving.set(true);
    this.store.updateBackpressureConfig(this.config()).subscribe({
      next: () => {
        this.toast.success('Backpressure configuration saved');
        this.saving.set(false);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to save configuration');
        this.saving.set(false);
      }
    });
  }
}
