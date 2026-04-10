import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { ApexStoreService, ApiToken, Permission } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-admin',
  standalone: true,
  imports: [FormsModule, DatePipe],
  templateUrl: './admin.component.html',
  styleUrl: './admin.component.scss'
})
export class AdminComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly allPermissions: Permission[] = ['Read', 'Write', 'Delete', 'Admin'];

  tokens        = signal<ApiToken[]>([]);
  loading       = signal(false);
  loadingCreate = signal(false);
  authError     = signal(false);
  createdToken  = signal<string | null>(null);
  newPerms      = signal<Permission[]>([]);
  hasSessionToken = signal(false);

  newName   = '';
  newExpiry: number | null = null;
  sessionToken = '';

  ngOnInit(): void {
    this.hasSessionToken.set(!!localStorage.getItem('apex_token'));
    this.load();
  }

  load(): void {
    this.loading.set(true);
    this.authError.set(false);
    this.store.listTokens().subscribe({
      next: (t) => { this.tokens.set(t); this.loading.set(false); },
      error: (e) => {
        if (e.status === 401 || e.status === 403) this.authError.set(true);
        else this.toast.error(e?.error?.message ?? 'Failed to load tokens');
        this.loading.set(false);
      }
    });
  }

  saveSessionToken(): void {
    const t = this.sessionToken.trim();
    if (!t) return;
    localStorage.setItem('apex_token', t);
    this.hasSessionToken.set(true);
    this.sessionToken = '';
    this.toast.success('Token salvo na sessão');
    this.load();
  }

  clearSessionToken(): void {
    localStorage.removeItem('apex_token');
    this.hasSessionToken.set(false);
    this.tokens.set([]);
    this.toast.info('Token removido');
  }

  togglePerm(p: Permission): void {
    this.newPerms.update(list =>
      list.includes(p) ? list.filter(x => x !== p) : [...list, p]
    );
  }

  createToken(): void {
    const name = this.newName.trim();
    if (!name || this.newPerms().length === 0) return;
    this.loadingCreate.set(true);
    this.createdToken.set(null);
    this.store.createToken(name, this.newPerms(), this.newExpiry ?? undefined).subscribe({
      next: (r) => {
        this.toast.success(`Token "${name}" criado!`);
        if (r.data?.token) this.createdToken.set(r.data.token);
        this.newName = ''; this.newExpiry = null; this.newPerms.set([]);
        this.load();
        this.loadingCreate.set(false);
      },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Create failed'); this.loadingCreate.set(false); }
    });
  }

  revokeToken(id: string): void {
    this.store.deleteToken(id).subscribe({
      next: () => { this.tokens.update(list => list.filter(t => t.id !== id)); this.toast.success('Token revogado'); },
      error: (e) => this.toast.error(e?.error?.message ?? 'Revoke failed')
    });
  }

  copyToken(): void {
    const t = this.createdToken();
    if (!t) return;
    navigator.clipboard.writeText(t).then(() => this.toast.success('Copiado!'));
  }

  nsToDate(ns: number): Date { return new Date(ns / 1_000_000); }
  isExpired(t: ApiToken): boolean { return !!t.expires_at && t.expires_at < Date.now() * 1_000_000; }
}
