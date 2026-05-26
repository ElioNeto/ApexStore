import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  LockPasswordIcon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  LockIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Policy {
  id: string;
  name: string;
  resource: string;
  actions: string[];
  effect: 'allow' | 'deny';
  priority: number;
  created_at: number;
}

@Component({
  selector: 'app-access-control',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './access-control.component.html',
  styleUrl: './access-control.component.scss'
})
export class AccessControlComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly LockPasswordIcon = LockPasswordIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly LockIcon = LockIcon;

  policies = signal<Policy[]>([]);
  loading = signal(false);
  creating = signal(false);

  newName = '';
  newResource = '';
  newActions = 'Read';
  newEffect: 'allow' | 'deny' = 'allow';
  newPriority = 100;

  actionOptions = ['Read', 'Write', 'Delete', 'Admin'];

  ngOnInit(): void { this.loadPolicies(); }

  loadPolicies(): void {
    this.loading.set(true);
    this.store.listPolicies().subscribe({
      next: (data) => {
        this.policies.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load policies');
        this.loading.set(false);
      }
    });
  }

  createPolicy(): void {
    if (!this.newName.trim() || !this.newResource.trim()) return;
    this.creating.set(true);
    this.store.createPolicy(this.newName.trim(), this.newResource.trim(), this.newActions.split(',').map(a => a.trim()), this.newEffect, this.newPriority).subscribe({
      next: () => {
        this.toast.success('Policy created!');
        this.newName = '';
        this.newResource = '';
        this.creating.set(false);
        this.loadPolicies();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create policy');
        this.creating.set(false);
      }
    });
  }

  deletePolicy(id: string, name: string): void {
    if (!confirm(`Delete policy "${name}"?`)) return;
    this.store.deletePolicy(id).subscribe({
      next: () => {
        this.toast.success(`Policy "${name}" deleted`);
        this.policies.update(list => list.filter(p => p.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete policy')
    });
  }

  toggleAction(action: string): void {
    const actions = this.newActions.split(',').map(a => a.trim()).filter(Boolean);
    if (actions.includes(action)) {
      this.newActions = actions.filter(a => a !== action).join(',');
    } else {
      this.newActions = [...actions, action].join(',');
    }
  }

  isActionSelected(action: string): boolean {
    return this.newActions.split(',').map(a => a.trim()).includes(action);
  }

  setEffect(effect: 'allow' | 'deny'): void {
    this.newEffect = effect;
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
