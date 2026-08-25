import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe, UpperCasePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  CpuIcon,
  DatabaseIcon,
  Share08Icon,
  ZapIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface CircuitBreaker {
  name: string;
  state: 'closed' | 'open' | 'half_open';
  failure_count: number;
  success_count: number;
  threshold: number;
  last_failure_at?: number;
}

interface HealthSummary {
  label: string;
  /// `unknown` is the state before the first probe returns.
  status: 'healthy' | 'degraded' | 'unhealthy' | 'unknown';
  message: string;
  icon: typeof RefreshIcon;
}

@Component({
  selector: 'app-resilience',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent, UpperCasePipe],
  templateUrl: './resilience.component.html',
  styleUrl: './resilience.component.scss'
})
export class ResilienceComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly Share08Icon = Share08Icon;
  readonly ZapIcon = ZapIcon;

  circuitBreakers = signal<CircuitBreaker[]>([]);
  healthSummary = signal<HealthSummary[]>([
    { label: 'API Server', status: 'unknown', message: 'Checking...', icon: CpuIcon },
    { label: 'Storage Engine', status: 'unknown', message: 'Checking...', icon: DatabaseIcon },
    { label: 'Network', status: 'unknown', message: 'Checking...', icon: Share08Icon },
  ]);
  loading = signal(false);
  lastCheck = signal<Date | null>(null);

  ngOnInit(): void { this.checkAll(); }

  checkAll(): void {
    this.loading.set(true);
    this.store.getCircuitBreakers().subscribe({
      next: (data) => {
        this.circuitBreakers.set(data);
        this.loading.set(false);
        this.lastCheck.set(new Date());
      },
      error: () => {
        this.toast.error('Failed to load circuit breaker state');
        this.loading.set(false);
      }
    });

    // Also check health for summary
    this.store.getHealth().subscribe({
      next: () => {
        this.healthSummary.set([
          { label: 'API Server', status: 'healthy', message: 'Responding normally', icon: CpuIcon },
          { label: 'Storage Engine', status: 'healthy', message: 'All operations nominal', icon: DatabaseIcon },
          { label: 'Network', status: 'healthy', message: 'Connectivity OK', icon: Share08Icon },
        ]);
      },
      error: () => {
        this.healthSummary.set([
          { label: 'API Server', status: 'unhealthy', message: 'Unreachable', icon: CpuIcon },
          { label: 'Storage Engine', status: 'degraded', message: 'Cannot verify', icon: DatabaseIcon },
          { label: 'Network', status: 'degraded', message: 'Connection lost', icon: Share08Icon },
        ]);
      }
    });
  }

  stateBadge(state: string): string {
    switch (state) {
      case 'closed': return 'badge-success';
      case 'open': return 'badge-danger';
      case 'half_open': return 'badge-warning';
      default: return 'badge-info';
    }
  }

  stateLabel(state: string): string {
    switch (state) {
      case 'closed': return 'Closed';
      case 'open': return 'Open';
      case 'half_open': return 'Half-Open';
      default: return state;
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
