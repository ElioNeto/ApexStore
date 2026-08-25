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
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface HealthProbe {
  name: string;
  endpoint: string;
  status: 'healthy' | 'unhealthy' | 'unknown';
  message: string;
  icon: typeof RefreshIcon;
}

@Component({
  selector: 'app-health',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent, UpperCasePipe],
  templateUrl: './health.component.html',
  styleUrl: './health.component.scss'
})
export class HealthComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly Share08Icon = Share08Icon;

  probes = signal<HealthProbe[]>([
    { name: 'Liveness', endpoint: '/health/live', status: 'unknown', message: 'Checking...', icon: CpuIcon },
    { name: 'Readiness', endpoint: '/health/ready', status: 'unknown', message: 'Checking...', icon: DatabaseIcon },
    { name: 'Startup', endpoint: '/health/startup', status: 'unknown', message: 'Checking...', icon: Share08Icon },
  ]);

  overall = signal<'healthy' | 'degraded' | 'unhealthy' | 'unknown'>('unknown');
  loading = signal(false);
  lastCheck = signal<Date | null>(null);

  ngOnInit(): void { this.checkAll(); }

  checkAll(): void {
    this.loading.set(true);
    this.store.getHealth().subscribe({
      next: () => {
        this.probes.update(list => list.map(p => ({
          ...p,
          status: 'healthy' as const,
          message: 'Endpoint responding normally',
        })));
        this.overall.set('healthy');
        this.lastCheck.set(new Date());
        this.loading.set(false);
        this.toast.success('All health probes passed');
      },
      error: () => {
        this.probes.update(list => list.map(p => ({
          ...p,
          status: 'unhealthy' as const,
          message: 'Health endpoint unreachable',
        })));
        this.overall.set('unhealthy');
        this.lastCheck.set(new Date());
        this.loading.set(false);
        this.toast.error('Health check failed - backend may be down');
      }
    });
  }

  checkProbe(probe: HealthProbe): void {
    this.probes.update(list => list.map(p =>
      p.name === probe.name ? { ...p, status: 'unknown' as const, message: 'Checking...' } : p
    ));

    this.store.getHealth().subscribe({
      next: () => {
        this.probes.update(list => list.map(p =>
          p.name === probe.name ? { ...p, status: 'healthy' as const, message: 'Endpoint responding' } : p
        ));
        this.updateOverall();
      },
      error: () => {
        this.probes.update(list => list.map(p =>
          p.name === probe.name ? { ...p, status: 'unhealthy' as const, message: 'Unreachable' } : p
        ));
        this.updateOverall();
      }
    });
  }

  private updateOverall(): void {
    const all = this.probes();
    const unhealthy = all.filter(p => p.status === 'unhealthy').length;
    if (unhealthy === 0) this.overall.set('healthy');
    else if (unhealthy < all.length) this.overall.set('degraded');
    else this.overall.set('unhealthy');
  }
}
