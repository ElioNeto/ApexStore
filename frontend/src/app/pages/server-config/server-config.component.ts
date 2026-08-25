import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Settings01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface ConfigEntry {
  key: string;
  value: string;
  type: 'string' | 'number' | 'boolean' | 'json';
  description: string;
  mutable: boolean;
  group: string;
}

@Component({
  selector: 'app-server-config',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './server-config.component.html',
  styleUrl: './server-config.component.scss'
})
export class ServerConfigComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Settings01Icon = Settings01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;

  config = signal<ConfigEntry[]>([]);
  filtered = signal<ConfigEntry[]>([]);
  loading = signal(false);
  saving = signal(false);
  filterText = '';

  ngOnInit(): void { this.loadConfig(); }

  loadConfig(): void {
    this.loading.set(true);
    this.store.getServerConfig().subscribe({
      next: (data) => {
        this.config.set(data);
        this.applyFilter();
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load server config');
        this.loading.set(false);
      }
    });
  }

  applyFilter(): void {
    const q = this.filterText.toLowerCase();
    if (!q) {
      this.filtered.set(this.config());
    } else {
      this.filtered.set(this.config().filter(c =>
        c.key.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q) ||
        c.group.toLowerCase().includes(q)
      ));
    }
  }

  getConfigGroups(): string[] {
    const groups = new Set(this.filtered().map(c => c.group));
    return Array.from(groups).sort();
  }

  /// Entries of one group. Templates cannot contain arrow functions -- Angular's
  /// parser reads `=>` as an assignment and rejects the binding -- so the
  /// per-group filter lives here rather than inline in the `@for`.
  configsInGroup(group: string): ConfigEntry[] {
    return this.filtered().filter(c => c.group === group);
  }

  updateConfig(entry: ConfigEntry, newValue: string): void {
    if (!entry.mutable) return;
    this.saving.set(true);
    this.store.updateServerConfig(entry.key, newValue).subscribe({
      next: () => {
        this.toast.success(`"${entry.key}" updated`);
        entry.value = newValue;
        this.saving.set(false);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to update config');
        this.saving.set(false);
      }
    });
  }

  onFilterChange(): void {
    this.applyFilter();
  }
}
