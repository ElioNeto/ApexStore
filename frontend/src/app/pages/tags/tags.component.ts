import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule, DatePipe } from '@angular/common';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  Tag01Icon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, Note } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-tags',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsIconComponent],
  templateUrl: './tags.component.html',
  styleUrl: './tags.component.scss'
})
export class TagsComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon = RefreshIcon;
  readonly Tag01Icon   = Tag01Icon;

  tags         = signal<string[]>([]);
  selectedTag  = signal<string | null>(null);
  tagNotes     = signal<Note[]>([]);
  loadingTags  = signal(false);
  loadingNotes = signal(false);

  ngOnInit(): void { this.loadTags(); }

  loadTags(): void {
    this.loadingTags.set(true);
    this.store.getTags().subscribe({
      next: (data) => { this.tags.set(data); this.loadingTags.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to load tags'); this.loadingTags.set(false); }
    });
  }

  selectTag(tag: string): void {
    this.selectedTag.set(tag);
    this.loadingNotes.set(true);
    this.tagNotes.set([]);
    this.store.getTagNotes(tag).subscribe({
      next: (notes) => { this.tagNotes.set(notes); this.loadingNotes.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to load notes for tag'); this.loadingNotes.set(false); }
    });
  }
}
