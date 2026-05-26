import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  Add01Icon,
  Delete01Icon,
  ArrowRight01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, Note } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface Snapshot {
  id: string;
  name: string;
  created_at: number;
  note_count: number;
}

@Component({
  selector: 'app-time-travel',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './time-travel.component.html',
  styleUrl: './time-travel.component.scss'
})
export class TimeTravelComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Share08Icon = Share08Icon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly ArrowRight01Icon = ArrowRight01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;

  snapshots = signal<Snapshot[]>([]);
  notes = signal<Note[]>([]);
  loading = signal(false);
  creating = signal(false);
  viewingNotes = signal(false);
  snapshotName = '';

  ngOnInit(): void { this.loadSnapshots(); }

  loadSnapshots(): void {
    this.loading.set(true);
    this.store.listSnapshots().subscribe({
      next: (data) => {
        this.snapshots.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load snapshots');
        this.loading.set(false);
      }
    });
  }

  createSnapshot(): void {
    const name = this.snapshotName.trim() || `snapshot-${Date.now()}`;
    this.creating.set(true);
    this.store.createSnapshot(name).subscribe({
      next: () => {
        this.toast.success(`Snapshot "${name}" created!`);
        this.snapshotName = '';
        this.creating.set(false);
        this.loadSnapshots();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Snapshot creation failed');
        this.creating.set(false);
      }
    });
  }

  viewSnapshot(snapshot: Snapshot): void {
    this.viewingNotes.set(true);
    this.store.getSnapshotNotes(snapshot.id).subscribe({
      next: (data) => {
        this.notes.set(data);
        this.viewingNotes.set(false);
      },
      error: () => {
        this.toast.error('Failed to load snapshot notes');
        this.viewingNotes.set(false);
      }
    });
  }

  restoreSnapshot(id: string, name: string): void {
    if (!confirm(`Restore snapshot "${name}"? This will overwrite current data.`)) return;
    this.store.restoreSnapshot(id).subscribe({
      next: () => {
        this.toast.success(`Snapshot "${name}" restored!`);
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Restore failed')
    });
  }

  deleteSnapshot(id: string, name: string): void {
    if (!confirm(`Delete snapshot "${name}"?`)) return;
    this.store.deleteSnapshot(id).subscribe({
      next: () => {
        this.toast.success(`Snapshot "${name}" deleted`);
        this.snapshots.update(list => list.filter(s => s.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Delete failed')
    });
  }

  closeNotes(): void {
    this.notes.set([]);
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
