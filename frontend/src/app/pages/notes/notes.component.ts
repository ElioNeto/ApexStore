import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  NoteEditIcon,
  Add01Icon,
  Delete01Icon,
  ArrowRight01Icon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, Note } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-notes',
  standalone: true,
  imports: [FormsModule, HugeiconsIconComponent],
  templateUrl: './notes.component.html',
  styleUrl: './notes.component.scss'
})
export class NotesComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly RefreshIcon       = RefreshIcon;
  readonly NoteEditIcon      = NoteEditIcon;
  readonly Add01Icon         = Add01Icon;
  readonly Delete01Icon      = Delete01Icon;
  readonly ArrowRight01Icon  = ArrowRight01Icon;

  notes          = signal<Note[]>([]);
  selectedNote   = signal<Note | null>(null);
  noteContent    = signal('');
  loadingList    = signal(false);
  loadingSave    = signal(false);
  newNotePath    = signal('');
  showNewInput   = signal(false);

  ngOnInit(): void { this.loadNotes(); }

  loadNotes(): void {
    this.loadingList.set(true);
    this.store.getNotes().subscribe({
      next: (data) => { this.notes.set(data); this.loadingList.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to load notes'); this.loadingList.set(false); }
    });
  }

  selectNote(note: Note): void {
    this.selectedNote.set(note);
    this.noteContent.set(note.content);
  }

  saveNote(): void {
    const note = this.selectedNote();
    if (!note) return;
    this.loadingSave.set(true);
    this.store.putNote(note.path, this.noteContent()).subscribe({
      next: () => {
        this.toast.success(`Note "${note.path}" saved`);
        this.selectedNote.update(n => n ? { ...n, content: this.noteContent() } : n);
        this.loadNotes();
        this.loadingSave.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to save note'); this.loadingSave.set(false); }
    });
  }

  deleteNote(): void {
    const note = this.selectedNote();
    if (!note) return;
    if (!confirm(`Delete note "${note.path}"?`)) return;
    this.store.deleteNote(note.path).subscribe({
      next: () => {
        this.toast.success(`Note "${note.path}" deleted`);
        this.selectedNote.set(null);
        this.noteContent.set('');
        this.loadNotes();
      },
      error: (e) => this.toast.error(e?.error?.message ?? 'Failed to delete note')
    });
  }

  createNote(): void {
    const path = this.newNotePath().trim();
    if (!path) return;
    this.loadingSave.set(true);
    this.store.putNote(path, '').subscribe({
      next: () => {
        this.toast.success(`Note "${path}" created`);
        this.newNotePath.set('');
        this.showNewInput.set(false);
        this.loadNotes();
        this.loadingSave.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Failed to create note'); this.loadingSave.set(false); }
    });
  }
}
