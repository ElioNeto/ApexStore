import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe, NgClass } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Upload01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  CpuIcon,
  PlayIcon,
  PauseIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface WasmPlugin {
  id: string;
  name: string;
  version: string;
  type: 'filter' | 'transform' | 'auth' | 'custom';
  status: 'active' | 'inactive' | 'error';
  size_kb: number;
  description: string;
  created_at: number;
}

@Component({
  selector: 'app-wasm-plugins',
  standalone: true,
  imports: [FormsModule, DatePipe, NgClass, HugeiconsIconComponent],
  templateUrl: './wasm-plugins.component.html',
  styleUrl: './wasm-plugins.component.scss'
})
export class WasmPluginsComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Upload01Icon = Upload01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;
  readonly PlayIcon = PlayIcon;
  readonly PauseIcon = PauseIcon;

  plugins = signal<WasmPlugin[]>([]);
  loading = signal(false);
  uploading = signal(false);
  toggling = signal<string | null>(null);

  ngOnInit(): void { this.loadPlugins(); }

  loadPlugins(): void {
    this.loading.set(true);
    this.store.listWasmPlugins().subscribe({
      next: (data) => {
        this.plugins.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load WASM plugins');
        this.loading.set(false);
      }
    });
  }

  onFileSelected(event: Event): void {
    const input = event.target as HTMLInputElement;
    if (!input.files?.length) return;
    const file = input.files[0];
    if (!file.name.endsWith('.wasm')) {
      this.toast.error('Please select a .wasm file');
      return;
    }
    this.uploading.set(true);
    this.store.uploadWasmPlugin(file).subscribe({
      next: () => {
        this.toast.success(`Plugin "${file.name}" uploaded`);
        this.uploading.set(false);
        this.loadPlugins();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Upload failed');
        this.uploading.set(false);
      }
    });
    input.value = '';
  }

  togglePlugin(id: string): void {
    this.toggling.set(id);
    this.store.toggleWasmPlugin(id).subscribe({
      next: () => {
        this.toast.success('Plugin toggled');
        this.toggling.set(null);
        this.loadPlugins();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to toggle plugin');
        this.toggling.set(null);
      }
    });
  }

  deletePlugin(id: string): void {
    if (!confirm('Delete this WASM plugin?')) return;
    this.store.deleteWasmPlugin(id).subscribe({
      next: () => {
        this.toast.success('Plugin deleted');
        this.plugins.update(list => list.filter(p => p.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete plugin')
    });
  }

  typeClass(type: string): string {
    switch (type) {
      case 'filter': return 'badge-info';
      case 'transform': return 'badge-warning';
      case 'auth': return 'badge-primary';
      default: return 'badge-default';
    }
  }

  statusClass(status: string): string {
    switch (status) {
      case 'active': return 'badge-success';
      case 'inactive': return 'badge-warning';
      default: return 'badge-danger';
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
