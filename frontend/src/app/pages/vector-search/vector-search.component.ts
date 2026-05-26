import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Search01Icon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  DatabaseIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface VectorIndex {
  name: string;
  dimension: number;
  metric: 'cosine' | 'euclidean' | 'dot';
  size: number;
  status: 'active' | 'building' | 'failed';
  created_at: number;
}

interface SearchResult {
  key: string;
  score: number;
  metadata?: Record<string, unknown>;
}

@Component({
  selector: 'app-vector-search',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './vector-search.component.html',
  styleUrl: './vector-search.component.scss'
})
export class VectorSearchComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Search01Icon = Search01Icon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly DatabaseIcon = DatabaseIcon;

  indexes = signal<VectorIndex[]>([]);
  loading = signal(false);
  creating = signal(false);

  searchQuery = '';
  searchResults = signal<SearchResult[]>([]);
  searching = signal(false);
  selectedIndex = '';

  newIndexName = '';
  newIndexDim = 128;
  newIndexMetric: 'cosine' | 'euclidean' | 'dot' = 'cosine';

  metrics: Array<{ value: string; label: string }> = [
    { value: 'cosine', label: 'Cosine' },
    { value: 'euclidean', label: 'Euclidean' },
    { value: 'dot', label: 'Dot Product' },
  ];

  ngOnInit(): void { this.loadIndexes(); }

  loadIndexes(): void {
    this.loading.set(true);
    this.store.listVectorIndexes().subscribe({
      next: (data) => {
        this.indexes.set(data);
        if (data.length > 0 && !this.selectedIndex) {
          this.selectedIndex = data[0].name;
        }
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load vector indexes');
        this.loading.set(false);
      }
    });
  }

  createIndex(): void {
    const name = this.newIndexName.trim();
    if (!name) return;
    this.creating.set(true);
    this.store.createVectorIndex(name, this.newIndexDim, this.newIndexMetric).subscribe({
      next: () => {
        this.toast.success(`Index "${name}" created!`);
        this.newIndexName = '';
        this.creating.set(false);
        this.loadIndexes();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create index');
        this.creating.set(false);
      }
    });
  }

  deleteIndex(name: string): void {
    if (!confirm(`Delete vector index "${name}"? All vectors will be lost.`)) return;
    this.store.deleteVectorIndex(name).subscribe({
      next: () => {
        this.toast.success(`Index "${name}" deleted`);
        this.indexes.update(list => list.filter(i => i.name !== name));
        if (this.selectedIndex === name) this.selectedIndex = '';
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete index')
    });
  }

  executeSearch(): void {
    if (!this.searchQuery.trim() || !this.selectedIndex) return;
    this.searching.set(true);
    this.store.vectorSearch(this.selectedIndex, this.searchQuery.trim()).subscribe({
      next: (results) => {
        this.searchResults.set(results);
        this.searching.set(false);
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Search failed');
        this.searching.set(false);
      }
    });
  }

  indexStatusClass(status: string): string {
    switch (status) {
      case 'active': return 'badge-success';
      case 'building': return 'badge-warning';
      default: return 'badge-danger';
    }
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
