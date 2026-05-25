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

interface LogLevel {
  module: string;
  level: 'trace' | 'debug' | 'info' | 'warn' | 'error' | 'off';
}

interface TelemetryConfig {
  sampling_rate: number;
  export_interval_sec: number;
  tracing_enabled: boolean;
  metrics_enabled: boolean;
  otlp_endpoint?: string;
}

@Component({
  selector: 'app-telemetry',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './telemetry.component.html',
  styleUrl: './telemetry.component.scss'
})
export class TelemetryComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Settings01Icon = Settings01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;

  config = signal<TelemetryConfig>({
    sampling_rate: 1.0,
    export_interval_sec: 60,
    tracing_enabled: true,
    metrics_enabled: true,
    otlp_endpoint: ''
  });

  logLevels = signal<LogLevel[]>([]);
  loading = signal(false);
  saving = signal(false);

  levelOptions = ['trace', 'debug', 'info', 'warn', 'error', 'off'];

  ngOnInit(): void { this.loadTelemetry(); }

  loadTelemetry(): void {
    this.loading.set(true);
    this.store.getTelemetryConfig().subscribe({
      next: (data) => {
        this.config.set(data.config);
        this.logLevels.set(data.log_levels);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load telemetry configuration');
        this.loading.set(false);
      }
    });
  }

  saveConfig(): void {
    this.saving.set(true);
    this.store.updateTelemetryConfig(this.config()).subscribe({
      next: () => {
        this.toast.success('Telemetry configuration saved');
        this.saving.set(false);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to save telemetry config');
        this.saving.set(false);
      }
    });
  }

  updateLogLevel(module: string, level: string): void {
    this.store.setLogLevel(module, level).subscribe({
      next: () => {
        this.toast.success(`Log level for "${module}" set to ${level}`);
        this.loadTelemetry();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to update log level')
    });
  }
}
