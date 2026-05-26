import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  BarChartIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface RateLimitEntry {
  name: string;
  limit: number;
  remaining: number;
  reset_at: number;
  window_sec: number;
}

@Component({
  selector: 'app-rate-limits',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './rate-limits.component.html',
  styleUrl: './rate-limits.component.scss'
})
export class RateLimitsComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly BarChartIcon = BarChartIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;

  limits = signal<RateLimitEntry[]>([]);
  loading = signal(false);
  lastRefresh = signal<Date | null>(null);

  ngOnInit(): void { this.loadLimits(); }

  loadLimits(): void {
    this.loading.set(true);
    this.store.getRateLimits().subscribe({
      next: (data) => {
        const entries = data['limits'] as RateLimitEntry[] ?? [];
        this.limits.set(entries);
        this.lastRefresh.set(new Date());
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load rate limits');
        this.loading.set(false);
      }
    });
  }

  usagePercent(entry: RateLimitEntry): number {
    if (entry.limit === 0) return 0;
    return ((entry.limit - entry.remaining) / entry.limit) * 100;
  }

  resetDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
