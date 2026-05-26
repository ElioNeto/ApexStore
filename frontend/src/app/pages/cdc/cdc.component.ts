import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  DatabaseIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface CDCTable {
  table: string;
  events: string[];
  status: 'active' | 'paused';
  since_lsn: string;
  last_event_at?: number;
}

interface CDCConfig {
  enabled: boolean;
  retention_hours: number;
  batch_size: number;
}

@Component({
  selector: 'app-cdc',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './cdc.component.html',
  styleUrl: './cdc.component.scss'
})
export class CdcComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly DatabaseIcon = DatabaseIcon;

  config = signal<CDCConfig>({ enabled: false, retention_hours: 24, batch_size: 1000 });
  tables = signal<CDCTable[]>([]);
  loading = signal(false);
  saving = signal(false);

  newTable = '';
  newTableEvents = 'insert,update,delete';

  allEvents = ['insert', 'update', 'delete', 'truncate'];

  ngOnInit(): void { this.loadCDC(); }

  loadCDC(): void {
    this.loading.set(true);
    this.store.getCDCConfig().subscribe({
      next: (data) => {
        this.config.set(data.config);
        this.tables.set(data.tables);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load CDC configuration');
        this.loading.set(false);
      }
    });
  }

  saveConfig(): void {
    this.saving.set(true);
    this.store.updateCDCConfig(this.config()).subscribe({
      next: () => {
        this.toast.success('CDC configuration updated');
        this.saving.set(false);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to save configuration');
        this.saving.set(false);
      }
    });
  }

  addTable(): void {
    const table = this.newTable.trim();
    if (!table) return;
    const events = this.newTableEvents.split(',').map(e => e.trim()).filter(Boolean);
    this.store.addCDCTable(table, events).subscribe({
      next: () => {
        this.toast.success(`Table "${table}" added to CDC`);
        this.newTable = '';
        this.loadCDC();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to add table')
    });
  }

  removeTable(table: string): void {
    if (!confirm(`Remove "${table}" from CDC tracking?`)) return;
    this.store.removeCDCTable(table).subscribe({
      next: () => {
        this.toast.success(`Table "${table}" removed`);
        this.tables.update(list => list.filter(t => t.table !== table));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to remove table')
    });
  }

  toggleEvent(event: string): void {
    const events = this.newTableEvents.split(',').map(e => e.trim()).filter(Boolean);
    if (events.includes(event)) {
      this.newTableEvents = events.filter(e => e !== event).join(',');
    } else {
      this.newTableEvents = [...events, event].join(',');
    }
  }

  isEventSelected(event: string): boolean {
    return this.newTableEvents.split(',').map(e => e.trim()).includes(event);
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
