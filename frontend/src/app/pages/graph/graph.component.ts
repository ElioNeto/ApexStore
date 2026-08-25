import { Component, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Share08Icon,
  NodeMoveDownIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, GraphData } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-graph',
  standalone: true,
  imports: [FormsModule, HugeiconsIconComponent],
  templateUrl: './graph.component.html',
  styleUrl: './graph.component.scss'
})
export class GraphComponent {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon      = RefreshIcon;
  readonly Share08Icon      = Share08Icon;
  readonly NodeMoveDownIcon = NodeMoveDownIcon;

  notePath    = signal('');
  depth       = signal<number>(1);
  graphData   = signal<GraphData | null>(null);
  loading     = signal(false);
  fetched     = signal(false);

  fetchGraph(): void {
    const path = this.notePath().trim();
    if (!path) return;

    this.loading.set(true);
    this.fetched.set(false);
    this.graphData.set(null);

    this.store.getGraphData(path, this.depth()).subscribe({
      next: (data) => {
        this.graphData.set(data);
        this.fetched.set(true);
        this.loading.set(false);
        if (data.nodes.length === 0) {
          this.toast.info('No graph data found for this note');
        } else {
          this.toast.success(`Found ${data.nodes.length} node(s) and ${data.edges.length} edge(s)`);
        }
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to fetch graph data');
        this.loading.set(false);
        this.fetched.set(true);
      }
    });
  }

  setDepth(d: number): void {
    this.depth.set(d);
  }
}
