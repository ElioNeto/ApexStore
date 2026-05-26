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
  ZapIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Webhook {
  id: string;
  url: string;
  events: string[];
  active: boolean;
  created_at: number;
  last_triggered_at?: number;
  last_status?: string;
}

@Component({
  selector: 'app-webhooks',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './webhooks.component.html',
  styleUrl: './webhooks.component.scss'
})
export class WebhooksComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly ZapIcon = ZapIcon;

  webhooks = signal<Webhook[]>([]);
  loading = signal(false);
  creating = signal(false);
  testing = signal<string | null>(null);

  newUrl = '';
  newEvents = 'key.set,key.delete';

  allEventOptions = ['key.set', 'key.delete', 'key.get', 'note.created', 'note.updated', 'note.deleted', 'flush', 'compact'];

  ngOnInit(): void { this.loadWebhooks(); }

  loadWebhooks(): void {
    this.loading.set(true);
    this.store.listWebhooks().subscribe({
      next: (data) => {
        this.webhooks.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load webhooks');
        this.loading.set(false);
      }
    });
  }

  createWebhook(): void {
    const url = this.newUrl.trim();
    if (!url) return;
    const events = this.newEvents.split(',').map(e => e.trim()).filter(Boolean);
    if (events.length === 0) return;

    this.creating.set(true);
    this.store.createWebhook(url, events).subscribe({
      next: () => {
        this.toast.success('Webhook created!');
        this.newUrl = '';
        this.creating.set(false);
        this.loadWebhooks();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create webhook');
        this.creating.set(false);
      }
    });
  }

  deleteWebhook(id: string): void {
    if (!confirm('Delete this webhook?')) return;
    this.store.deleteWebhook(id).subscribe({
      next: () => {
        this.toast.success('Webhook deleted');
        this.webhooks.update(list => list.filter(w => w.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete webhook')
    });
  }

  testWebhook(id: string): void {
    this.testing.set(id);
    this.store.testWebhook(id).subscribe({
      next: () => {
        this.toast.success('Test webhook sent!');
        this.testing.set(null);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Webhook test failed');
        this.testing.set(null);
      }
    });
  }

  toggleEvent(event: string): void {
    const events = this.newEvents.split(',').map(e => e.trim()).filter(Boolean);
    if (events.includes(event)) {
      this.newEvents = events.filter(e => e !== event).join(',');
    } else {
      this.newEvents = [...events, event].join(',');
    }
  }

  isEventSelected(event: string): boolean {
    return this.newEvents.split(',').map(e => e.trim()).includes(event);
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
