import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Add01Icon,
  Delete01Icon,
  CheckmarkCircle01Icon,
  CancelCircleIcon,
  CpuIcon,
  PlayIcon,
  DatabaseIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

interface CICDFixture {
  id: string;
  name: string;
  description: string;
  type: 'data' | 'config' | 'schema';
  size: string;
  created_at: number;
}

interface TestDataGeneration {
  status: 'idle' | 'generating' | 'completed' | 'failed';
  records_generated: number;
  elapsed_ms?: number;
}

@Component({
  selector: 'app-cicd',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './cicd.component.html',
  styleUrl: './cicd.component.scss'
})
export class CicdComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Add01Icon = Add01Icon;
  readonly Delete01Icon = Delete01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly CancelCircleIcon = CancelCircleIcon;
  readonly CpuIcon = CpuIcon;
  readonly PlayIcon = PlayIcon;
  readonly DatabaseIcon = DatabaseIcon;

  fixtures = signal<CICDFixture[]>([]);
  loading = signal(false);
  generating = signal(false);
  creating = signal(false);

  genStatus = signal<TestDataGeneration>({ status: 'idle', records_generated: 0 });
  newFixtureName = '';
  newFixtureDesc = '';
  newFixtureType: 'data' | 'config' | 'schema' = 'data';

  fixtureTypes: Array<{ value: string; label: string }> = [
    { value: 'data', label: 'Test Data' },
    { value: 'config', label: 'Configuration' },
    { value: 'schema', label: 'Schema' },
  ];

  genCount = 1000;

  ngOnInit(): void { this.loadFixtures(); }

  loadFixtures(): void {
    this.loading.set(true);
    this.store.listCICDFixtures().subscribe({
      next: (data) => {
        this.fixtures.set(data);
        this.loading.set(false);
      },
      error: () => {
        this.toast.error('Failed to load CI/CD fixtures');
        this.loading.set(false);
      }
    });
  }

  createFixture(): void {
    if (!this.newFixtureName.trim()) return;
    this.creating.set(true);
    this.store.createCICDFixture(this.newFixtureName.trim(), this.newFixtureDesc.trim(), this.newFixtureType).subscribe({
      next: () => {
        this.toast.success('Fixture created!');
        this.newFixtureName = '';
        this.newFixtureDesc = '';
        this.creating.set(false);
        this.loadFixtures();
      },
      error: (e) => {
        this.toast.error(e?.error?.message ?? 'Failed to create fixture');
        this.creating.set(false);
      }
    });
  }

  deleteFixture(id: string, name: string): void {
    if (!confirm(`Delete fixture "${name}"?`)) return;
    this.store.deleteCICDFixture(id).subscribe({
      next: () => {
        this.toast.success('Fixture deleted');
        this.fixtures.update(list => list.filter(f => f.id !== id));
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete fixture')
    });
  }

  generateTestData(): void {
    this.generating.set(true);
    this.genStatus.set({ status: 'generating', records_generated: 0 });
    this.store.generateTestData(this.genCount).subscribe({
      next: (result) => {
        this.genStatus.set({ status: 'completed', records_generated: result.records_generated, elapsed_ms: result.elapsed_ms });
        this.generating.set(false);
        this.toast.success(`Generated ${result.records_generated} records in ${result.elapsed_ms}ms`);
      },
      error: (e) => {
        this.genStatus.set({ status: 'failed', records_generated: 0 });
        this.generating.set(false);
        this.toast.error(e?.error?.message ?? 'Data generation failed');
      }
    });
  }

  nsToDate(ns: number): Date {
    return new Date(ns / 1_000_000);
  }
}
