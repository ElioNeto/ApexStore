import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  TickDouble01Icon,
  Cancel01Icon,
  Flag01Icon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, FeatureFlag } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-features',
  standalone: true,
  imports: [FormsModule, HugeiconsIconComponent],
  templateUrl: './features.component.html',
  styleUrl: './features.component.scss'
})
export class FeaturesComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly TickDouble01Icon = TickDouble01Icon;
  readonly Cancel01Icon = Cancel01Icon;
  readonly Flag01Icon = Flag01Icon;

  flags = signal<FeatureFlag[]>([]);
  version = signal<number | null>(null);
  loading = signal(false);
  loadingNew = signal(false);
  newName = '';
  newDesc = '';

  ngOnInit(): void { this.load(); }

  load(): void {
    this.loading.set(true);
    this.store.listFeatures().subscribe({
      next: (r) => { this.flags.set(r.features); this.version.set(r.version); this.loading.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to load features'); this.loading.set(false); }
    });
  }

  createFlag(enabled: boolean): void {
    const name = this.newName.trim(); if (!name) return;
    this.loadingNew.set(true);
    this.store.setFeature(name, enabled, this.newDesc.trim()).subscribe({
      next: () => { this.toast.success(`Flag "${name}" ${enabled ? 'enabled' : 'disabled'}`); this.newName = ''; this.newDesc = ''; this.load(); this.loadingNew.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed'); this.loadingNew.set(false); }
    });
  }

  toggle(flag: FeatureFlag, enabled: boolean): void {
    this.store.setFeature(flag.name, enabled, flag.description).subscribe({
      next: () => { this.toast.success(`"${flag.name}" ${enabled ? 'enabled' : 'disabled'}`); this.flags.update(l => l.map(f => f.name === flag.name ? { ...f, enabled } : f)); },
      error: (e) => this.toast.error(e?.error?.message ?? 'Toggle failed')
    });
  }
}
