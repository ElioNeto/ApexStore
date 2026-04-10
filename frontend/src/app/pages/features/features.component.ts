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
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Feature Flags</h1>
          <p class="page-subtitle">Ative ou desative features em runtime sem restart</p>
        </div>
        <div style="display:flex;align-items:center;gap:12px">
          @if (version() !== null) {
            <span class="badge badge-info">version {{ version() }}</span>
          }
          <button class="btn btn-secondary" (click)="load()" [disabled]="loading()">
            @if (loading()) { <span class="spinner"></span> }
            @else { <hugeicons-icon [icon]="RefreshIcon" [size]="16" [strokeWidth]="1.5" /> }
            Refresh
          </button>
        </div>
      </div>

      <!-- New flag form -->
      <div class="card" style="margin-bottom:24px">
        <div class="card-header">
          <span class="op-badge">POST</span>
          <span class="card-title">Create / Update Flag</span>
        </div>
        <div class="card-body">
          <div class="form-row">
            <div class="input-group" style="flex:1">
              <label>Flag name</label>
              <input [(ngModel)]="newName" placeholder="ex: dark_mode" />
            </div>
            <div class="input-group" style="flex:2">
              <label>Description</label>
              <input [(ngModel)]="newDesc" placeholder="optional description" />
            </div>
            <div style="display:flex;gap:8px;align-self:flex-end">
              <button class="btn btn-success" [disabled]="!newName.trim()||loadingNew()" (click)="createFlag(true)">
                @if (loadingNew()) { <span class="spinner"></span> }
                @else { <hugeicons-icon [icon]="TickDouble01Icon" [size]="15" [strokeWidth]="1.5" /> }
                Enable
              </button>
              <button class="btn btn-danger" [disabled]="!newName.trim()||loadingNew()" (click)="createFlag(false)">
                <hugeicons-icon [icon]="Cancel01Icon" [size]="15" [strokeWidth]="1.5" />
                Disable
              </button>
            </div>
          </div>
        </div>
      </div>

      @if (loading() && flags().length === 0) {
        <div class="loading-state"><span class="spinner" style="width:28px;height:28px;border-width:3px"></span></div>
      } @else if (flags().length === 0) {
        <div class="empty-state">Nenhuma feature flag configurada ainda.</div>
      } @else {
        <div class="flags-grid">
          @for (flag of flags(); track flag.name) {
            <div class="flag-card" [class.flag-enabled]="flag.enabled" [class.flag-disabled]="!flag.enabled">
              <div class="flag-header">
                <div style="display:flex;align-items:center;gap:8px">
                  <hugeicons-icon [icon]="Flag01Icon" [size]="16" [strokeWidth]="1.5" />
                  <span class="flag-name">{{ flag.name }}</span>
                </div>
                <span class="badge" [class.badge-success]="flag.enabled" [class.badge-danger]="!flag.enabled">
                  {{ flag.enabled ? 'ENABLED' : 'DISABLED' }}
                </span>
              </div>
              @if (flag.description) {
                <p class="flag-desc">{{ flag.description }}</p>
              }
              <div class="flag-actions">
                @if (!flag.enabled) {
                  <button class="btn btn-success btn-sm" (click)="toggle(flag, true)">
                    <hugeicons-icon [icon]="TickDouble01Icon" [size]="14" [strokeWidth]="1.5" /> Enable
                  </button>
                } @else {
                  <button class="btn btn-danger btn-sm" (click)="toggle(flag, false)">
                    <hugeicons-icon [icon]="Cancel01Icon" [size]="14" [strokeWidth]="1.5" /> Disable
                  </button>
                }
              </div>
            </div>
          }
        </div>
      }
    </div>
  `,
  styles: [`
    .page { padding:32px; max-width:1000px; }
    .page-header { display:flex; align-items:flex-start; justify-content:space-between; margin-bottom:28px; gap:16px; flex-wrap:wrap; }
    .page-title { font-size:1.6rem; font-weight:700; }
    .page-subtitle { color:var(--text-muted); font-size:0.9rem; margin-top:4px; }
    .card { background:var(--bg-card); border:1px solid var(--border); border-radius:var(--radius-lg); overflow:hidden; }
    .card-header { display:flex; align-items:center; gap:12px; padding:14px 18px; border-bottom:1px solid var(--border); background:var(--bg-secondary); }
    .card-title { font-weight:600; font-size:0.9rem; }
    .card-body { padding:18px; }
    .form-row { display:flex; gap:12px; align-items:flex-end; flex-wrap:wrap; }
    .op-badge { display:inline-block; padding:3px 8px; border-radius:5px; font-size:0.7rem; font-weight:700; font-family:var(--font-mono); background:var(--accent-dim); color:var(--accent); }
    .loading-state { display:flex; justify-content:center; padding:60px; }
    .empty-state { padding:60px; text-align:center; color:var(--text-muted); font-size:0.9rem; }
    .flags-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(280px,1fr)); gap:16px; }
    .flag-card { background:var(--bg-card); border:1px solid var(--border); border-radius:var(--radius-lg); padding:18px; display:flex; flex-direction:column; gap:10px; transition:border-color 0.15s; }
    .flag-card.flag-enabled { border-left:3px solid var(--green); }
    .flag-card.flag-disabled { border-left:3px solid var(--text-muted); }
    .flag-header { display:flex; align-items:center; justify-content:space-between; gap:8px; }
    .flag-name { font-family:var(--font-mono); font-weight:600; font-size:0.9rem; color:var(--text-primary); }
    .flag-desc { font-size:0.82rem; color:var(--text-secondary); margin:0; }
    .flag-actions { margin-top:4px; }
  `]
})
export class FeaturesComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon      = RefreshIcon;
  readonly TickDouble01Icon = TickDouble01Icon;
  readonly Cancel01Icon     = Cancel01Icon;
  readonly Flag01Icon       = Flag01Icon;

  flags      = signal<FeatureFlag[]>([]);
  version    = signal<number | null>(null);
  loading    = signal(false);
  loadingNew = signal(false);
  newName    = '';
  newDesc    = '';

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
