import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe, NgClass } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Add01Icon,
  Delete01Icon,
  PlayIcon,
  PauseIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  CpuIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface ChaosExperiment {
  id: string;
  name: string;
  type: 'latency' | 'error' | 'crash' | 'partition' | 'resource';
  target: string;
  status: 'running' | 'stopped' | 'completed' | 'failed';
  config: Record<string, unknown>;
  created_at: number;
}

@Component({
  selector: 'app-chaos',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent, NgClass],
  templateUrl: './chaos.component.html',
  styleUrl: './chaos.component.scss'
})
export class ChaosComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly PlayIcon = PlayIcon;
  readonly PauseIcon = PauseIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;

  experiments = signal<ChaosExperiment[]>([]);
  loading = signal(false);
  creating = signal(false);
  toggling = signal<string | null>(null);

  newExp = {
    name: '',
    type: 'latency' as ChaosExperiment['type'],
    target: '',
    config: '{}'
  };

  expTypes: Array<{ value: string; label: string }> = [
    { value: 'latency', label: 'Latency Injection' },
    { value: 'error', label: 'Error Injection' },
    { value: 'crash', label: 'Crash' },
    { value: 'partition', label: 'Network Partition' },
    { value: 'resource', label: 'Resource Exhaustion' },
  ];

  ngOnInit(): void { this.loadExperiments(); }

  loadExperiments(): void {
    this.loading.set(true);
    this.store.listChaosExperiments().subscribe({
      next: (data) => {
        this.experiments.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load chaos experiments');
        this.loading.set(false);
      }
    });
  }

  createExperiment(): void {
    if (!this.newExp.name.trim() || !this.newExp.target.trim()) return;
    this.creating.set(true);
    let config: Record<string, unknown> = {};
    try { config = JSON.parse(this.newExp.config); } catch { config = {}; }
    this.store.createChaosExperiment(this.newExp.name.trim(), this.newExp.type, this.newExp.target.trim(), config).subscribe({
      next: () => {
        this.toast.success('Chaos experiment created!');
        this.newExp = { name: '', type: 'latency', target: '', config: '{}' };
        this.creating.set(false);
        this.loadExperiments();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create experiment');
        this.creating.set(false);
      }
    });
  }

  toggleExperiment(id: string): void {
    this.toggling.set(id);
    this.store.toggleChaosExperiment(id).subscribe({
      next: () => {
        this.toast.success('Experiment toggled');
        this.toggling.set(null);
        this.loadExperiments();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to toggle experiment');
        this.toggling.set(null);
      }
    });
  }

  deleteExperiment(id: string): void {
    if (!confirm('Delete this chaos experiment?')) return;
    this.store.deleteChaosExperiment(id).subscribe({
      next: () => {
        this.toast.success('Experiment deleted');
        this.experiments.update(list => list.filter(e => e.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete experiment')
    });
  }

  typeClass(type: string): string {
    switch (type) {
      case 'latency': return 'badge-warning';
      case 'error': return 'badge-danger';
      case 'crash': return 'badge-danger';
      case 'partition': return 'badge-info';
      case 'resource': return 'badge-warning';
      default: return 'badge-info';
    }
  }

  statusClass(status: string): string {
    switch (status) {
      case 'running': return 'badge-danger';
      case 'stopped': return 'badge-warning';
      case 'completed': return 'badge-success';
      default: return 'badge-info';
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
