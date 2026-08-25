import { Component, computed, inject, signal, OnInit, OnDestroy } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  Time04Icon,
  DatabaseIcon,
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
  readonly Time04Icon = Time04Icon;
  readonly DatabaseIcon = DatabaseIcon;
  readonly ArrowRight01Icon = ArrowRight01Icon;

  deviceId = signal('');
  syncStatus = this.sync.status;
  lastSync = this.sync.lastSyncTimestamp;
  pending = this.sync.pendingChanges;
  lastError = this.sync.lastError;
  autoRefresh = signal(false);
  /// Derived from the service signal. This used to be a writable signal fed by
  /// `lastSyncTimestamp.subscribe(...)`, but `lastSyncTimestamp` is a signal and
  /// signals have no `subscribe`, so the callback never ran and the label was
  /// permanently "Never".
  readonly lastSyncedLabel = computed(() => {
    const ts = this.sync.lastSyncTimestamp();
    return ts ? new Date(ts).toLocaleString() : 'Never';
  });
  private refreshTimer: ReturnType<typeof setInterval> | null = null;

  statusIcon(status: SyncStatus): typeof RefreshIcon {
    switch (status) {
      case 'connected': return CheckmarkCircle01Icon;
      case 'connecting': return Time04Icon;
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
    // `lastSyncedLabel` is a computed signal now; nothing to wire up here.
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

  // The service's signals drive the template directly, so there is nothing to
  // poll: Angular re-renders when `lastSyncTimestamp` changes. The timer that
  // used to live here called `lastSyncedLabel.update(l => l)`, which wrote the
  // same value back and therefore notified nothing, since signals compare for
  // equality before propagating.
  private startAutoRefresh(): void {
    this.stopAutoRefresh();
  }

  private stopAutoRefresh(): void {
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }
  }
}
