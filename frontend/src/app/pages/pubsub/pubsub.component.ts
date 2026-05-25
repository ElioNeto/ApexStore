import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  ArrowRight01Icon,
  PackageIcon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Topic {
  name: string;
  message_count: number;
  last_message_at?: number;
}

interface Subscription {
  id: string;
  topic: string;
  endpoint: string;
  active: boolean;
}

@Component({
  selector: 'app-pubsub',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './pubsub.component.html',
  styleUrl: './pubsub.component.scss'
})
export class PubsubComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly ArrowRight01Icon = ArrowRight01Icon;
  readonly PackageIcon = PackageIcon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;

  topics = signal<Topic[]>([]);
  subscriptions = signal<Subscription[]>([]);
  loading = signal(false);
  publishing = signal(false);

  publishTopic = '';
  publishMessage = '';
  selectedTopic = signal<string | null>(null);

  ngOnInit(): void { this.loadData(); }

  loadData(): void {
    this.loading.set(true);
    this.store.listTopics().subscribe({
      next: (data) => {
        this.topics.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load topics');
        this.loading.set(false);
      }
    });
  }

  loadSubscriptions(topic: string): void {
    this.selectedTopic.set(topic);
    this.store.listSubscriptions(topic).subscribe({
      next: (data) => {
        this.subscriptions.set(data);
      },
      error: () => {
        this.toast.error('Failed to load subscriptions');
      }
    });
  }

  publishMessage(): void {
    const topic = this.publishTopic.trim();
    const message = this.publishMessage.trim();
    if (!topic || !message) return;

    this.publishing.set(true);
    this.store.publishMessage(topic, message).subscribe({
      next: () => {
        this.toast.success(`Message published to "${topic}"!`);
        this.publishMessage = '';
        this.publishing.set(false);
        this.loadData();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Publish failed');
        this.publishing.set(false);
      }
    });
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
