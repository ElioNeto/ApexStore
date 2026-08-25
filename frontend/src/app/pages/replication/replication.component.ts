import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe, NgClass, UpperCasePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  DatabaseIcon,
  CpuIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface ReplicaNode {
  id: string;
  name: string;
  role: 'primary' | 'secondary' | 'observer';
  status: 'online' | 'offline' | 'syncing';
  lag_ms: number;
  address: string;
  last_heartbeat: number;
}

interface ReplicationSummary {
  connected: number;
  total: number;
  avg_lag_ms: number;
  status: 'healthy' | 'degraded' | 'critical';
}

@Component({
  selector: 'app-replication',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent, NgClass, UpperCasePipe],
  templateUrl: './replication.component.html',
  styleUrl: './replication.component.scss'
})
export class ReplicationComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly CpuIcon = CpuIcon;

  nodes = signal<ReplicaNode[]>([]);
  summary = signal<ReplicationSummary | null>(null);
  loading = signal(false);

  ngOnInit(): void { this.loadTopology(); }

  loadTopology(): void {
    this.loading.set(true);
    this.store.getReplicationTopology().subscribe({
      next: (data) => {
        this.nodes.set(data.nodes);
        this.summary.set(data.summary);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load replication topology');
        this.loading.set(false);
      }
    });
  }

  promoteToPrimary(id: string): void {
    if (!confirm('Promote this node to primary?')) return;
    this.store.promoteReplica(id).subscribe({
      next: () => {
        this.toast.success('Node promoted to primary');
        this.loadTopology();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Promotion failed')
    });
  }

  removeNode(id: string): void {
    if (!confirm('Remove this node from the cluster?')) return;
    this.store.removeReplica(id).subscribe({
      next: () => {
        this.toast.success('Node removed');
        this.nodes.update(list => list.filter(n => n.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to remove node')
    });
  }

  statusClass(status: string): string {
    switch (status) {
      case 'online': return 'badge-success';
      case 'syncing': return 'badge-warning';
      default: return 'badge-danger';
    }
  }

  roleClass(role: string): string {
    switch (role) {
      case 'primary': return 'badge-primary';
      case 'secondary': return 'badge-info';
      default: return 'badge-default';
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
