import { Component, inject, signal, OnInit, OnDestroy } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  Time05Icon,
  DataIcon,
  ArrowRight01Icon,
} from '@hugeicons/core-free-icons';
import { SyncService, SyncStatus } from '../../services/sync.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-sync-status',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './sync-status.component.html',
  styleUrl: './sync-status.component.scss',
})
export class SyncStatusComponent implements OnInit, OnDestroy {
  private sync = inject(SyncService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly Time05Icon = Time05Icon;
  readonly DataIcon = DataIcon;
  readonly ArrowRight01Icon = ArrowRight01Icon;

  deviceId = signal('');
  syncStatus = this.sync.status;
  lastSync = this.sync.lastSyncTimestamp;
  pending = this.sync.pendingChanges;
  lastError = this.sync.lastError;
  autoRefresh = signal(false);
  lastSyncedLabel = signal<string>('Never');
  private refreshTimer: ReturnType<typeof setInterval> | null = null;

  statusIcon(status: SyncStatus): typeof RefreshIcon {
    switch (status) {
      case 'connected': return CheckmarkCircle01Icon;
      case 'connecting': return Time05Icon;
      case 'error': return CancelCircleIcon;
      default: return CancelCircleIcon;
    }
  }

  statusColor(status: SyncStatus): string {
    switch (status) {
      case 'connected': return 'var(--color-success)';
      case 'connecting': return 'var(--color-warning)';
      case 'error': return 'var(--color-error)';
      default: return 'var(--color-muted)';
    }
  }

  ngOnInit(): void {
    this.sync.lastSyncTimestamp.subscribe(ts => {
      if (ts) {
        this.lastSyncedLabel.set(new Date(ts).toLocaleString());
      }
    });
  }

  ngOnDestroy(): void {
    this.stopAutoRefresh();
  }

  connect(): void {
    const id = this.deviceId().trim();
    if (!id) {
      this.toast.error('Please enter a device ID');
      return;
    }
    this.sync.connect(id);
    this.toast.info(`Connecting as "${id}"...`);
    if (this.autoRefresh()) {
      this.startAutoRefresh();
    }
  }

  disconnect(): void {
    this.sync.disconnect();
    this.toast.info('Disconnected');
    this.stopAutoRefresh();
  }

  private startAutoRefresh(): void {
    this.stopAutoRefresh();
    this.refreshTimer = setInterval(() => {
      // Trigger a status refresh by re-checking signals
      this.lastSyncedLabel.update(l => l);
    }, 5000);
  }

  private stopAutoRefresh(): void {
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }
}
