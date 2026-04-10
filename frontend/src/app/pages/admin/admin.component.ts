import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { HugeiconsComponent } from '@hugeicons/angular';
import {
  RefreshIcon,
  LockPasswordIcon,
  Add01Icon,
  Delete01Icon,
  Copy01Icon,
  CheckmarkCircle01Icon,
  LockIcon,
} from '@hugeicons/core-free-icons';
import { ApexStoreService, ApiToken, Permission } from '../../services/apex-store.service';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-admin',
  standalone: true,
  imports: [FormsModule, DatePipe, HugeiconsComponent],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1 class="page-title">Token Admin</h1>
          <p class="page-subtitle">Gerenciamento de Bearer Tokens — requer <code>API_AUTH_ENABLED=true</code></p>
        </div>
        <button class="btn btn-secondary" (click)="load()" [disabled]="loading()">
          <hugeicons-icon [icon]="RefreshIcon" [size]="16" [strokeWidth]="1.5" /> Refresh
        </button>
      </div>

      <!-- Session token -->
      <div class="card" style="margin-bottom:24px">
        <div class="card-header">
          <hugeicons-icon [icon]="LockPasswordIcon" [size]="17" [strokeWidth]="1.5" />
          <span class="card-title">Bearer Token da Sessão</span>
        </div>
        <div class="card-body">
          <p style="font-size:0.85rem;color:var(--text-secondary);margin-bottom:12px">
            Informe o token para autenticar todas as requisições desta sessão.
          </p>
          <div class="row-inline">
            <div class="input-group" style="flex:1">
              <label>Token</label>
              <input type="password" [(ngModel)]="sessionToken" placeholder="apx_..." />
            </div>
            <button class="btn btn-primary" style="align-self:flex-end" (click)="saveSessionToken()">
              <hugeicons-icon [icon]="CheckmarkCircle01Icon" [size]="15" [strokeWidth]="1.5" /> Salvar
            </button>
            @if (hasSessionToken()) {
              <button class="btn btn-danger" style="align-self:flex-end" (click)="clearSessionToken()">
                <hugeicons-icon [icon]="Delete01Icon" [size]="15" [strokeWidth]="1.5" /> Limpar
              </button>
            }
          </div>
          @if (hasSessionToken()) {
            <div class="token-active-badge">
              <hugeicons-icon [icon]="CheckmarkCircle01Icon" [size]="14" [strokeWidth]="1.5" /> Token ativo na sessão
            </div>
          }
        </div>
      </div>

      <!-- Create token -->
      <div class="card" style="margin-bottom:24px">
        <div class="card-header"><span class="op-badge">POST</span><span class="card-title">Criar Token</span></div>
        <div class="card-body">
          <div class="form-grid">
            <div class="input-group">
              <label>Nome</label>
              <input [(ngModel)]="newName" placeholder="ex: ci-pipeline" />
            </div>
            <div class="input-group">
              <label>Expira em (dias) — vazio = nunca</label>
              <input type="number" [(ngModel)]="newExpiry" placeholder="30" min="1" />
            </div>
            <div class="input-group" style="grid-column:1/-1">
              <label>Permissions</label>
              <div class="perms-row">
                @for (p of allPermissions; track p) {
                  <label class="perm-toggle" [class.selected]="newPerms().includes(p)">
                    <input type="checkbox" [checked]="newPerms().includes(p)" (change)="togglePerm(p)" />
                    {{ p }}
                  </label>
                }
              </div>
            </div>
          </div>
          <button class="btn btn-primary" style="margin-top:14px" [disabled]="!newName.trim()||newPerms().length===0||loadingCreate()" (click)="createToken()">
            @if (loadingCreate()) { <span class="spinner"></span> }
            @else { <hugeicons-icon [icon]="Add01Icon" [size]="15" [strokeWidth]="1.5" /> }
            Create Token
          </button>

          @if (createdToken()) {
            <div class="created-token-box">
              <div class="created-token-label">
                <hugeicons-icon [icon]="LockIcon" [size]="14" [strokeWidth]="1.5" />
                Copie agora — este token não será exibido novamente
              </div>
              <div class="created-token-value">{{ createdToken() }}</div>
              <button class="btn btn-secondary btn-sm" (click)="copyToken()">
                <hugeicons-icon [icon]="Copy01Icon" [size]="14" [strokeWidth]="1.5" /> Copy
              </button>
            </div>
          }
        </div>
      </div>

      <!-- Tokens list -->
      <div class="card">
        <div class="card-header" style="justify-content:space-between">
          <div style="display:flex;align-items:center;gap:10px">
            <span class="card-title">Tokens Ativos</span>
            <span class="badge badge-info">{{ tokens().length }}</span>
          </div>
        </div>

        @if (loading() && tokens().length === 0) {
          <div class="loading-state"><span class="spinner" style="width:28px;height:28px;border-width:3px"></span></div>
        } @else if (authError()) {
          <div class="auth-error">
            <hugeicons-icon [icon]="LockIcon" [size]="24" [strokeWidth]="1.5" />
            <div>
              <strong>Auth required</strong>
              <p>Configure o Bearer Token da sessão acima e tente novamente.</p>
            </div>
          </div>
        } @else if (tokens().length === 0) {
          <div class="empty-state">Nenhum token criado ainda.</div>
        } @else {
          <div class="table-wrapper">
            <table class="tokens-table">
              <thead>
                <tr><th>Name</th><th>ID</th><th>Permissions</th><th>Created</th><th>Expires</th><th>Actions</th></tr>
              </thead>
              <tbody>
                @for (t of tokens(); track t.id) {
                  <tr [class.expired]="isExpired(t)">
                    <td class="name-cell">{{ t.name }}</td>
                    <td class="mono id-cell">{{ t.id.substring(0, 8) }}...</td>
                    <td>
                      <div class="perms-chips">
                        @for (p of t.permissions; track p) {
                          <span class="perm-chip" [class]="'perm-' + p.toLowerCase()">{{ p }}</span>
                        }
                      </div>
                    </td>
                    <td class="time-cell">{{ nsToDate(t.created_at) | date:'dd/MM/yy HH:mm' }}</td>
                    <td class="time-cell">
                      @if (t.expires_at) { {{ nsToDate(t.expires_at) | date:'dd/MM/yy' }} }
                      @else { <span style="color:var(--text-muted)">Never</span> }
                    </td>
                    <td>
                      <button class="btn btn-danger btn-sm" (click)="revokeToken(t.id)" style="display:flex;align-items:center;gap:6px">
                        <hugeicons-icon [icon]="Delete01Icon" [size]="14" [strokeWidth]="1.5" /> Revoke
                      </button>
                    </td>
                  </tr>
                }
              </tbody>
            </table>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    .page { padding:32px; max-width:1100px; }
    .page-header { display:flex; align-items:flex-start; justify-content:space-between; margin-bottom:28px; gap:16px; flex-wrap:wrap; }
    .page-title { font-size:1.6rem; font-weight:700; }
    .page-subtitle { color:var(--text-muted); font-size:0.9rem; margin-top:4px; }
    .page-subtitle code { font-family:var(--font-mono); background:var(--bg-secondary); padding:2px 6px; border-radius:4px; font-size:0.8rem; }
    .card { background:var(--bg-card); border:1px solid var(--border); border-radius:var(--radius-lg); overflow:hidden; }
    .card-header { display:flex; align-items:center; gap:12px; padding:14px 18px; border-bottom:1px solid var(--border); background:var(--bg-secondary); }
    .card-title { font-weight:600; font-size:0.9rem; }
    .card-body { padding:20px; }
    .row-inline { display:flex; gap:10px; align-items:flex-end; flex-wrap:wrap; }
    .op-badge { display:inline-block; padding:3px 8px; border-radius:5px; font-size:0.7rem; font-weight:700; font-family:var(--font-mono); background:var(--accent-dim); color:var(--accent); }
    .form-grid { display:grid; grid-template-columns:1fr 1fr; gap:14px; }
    .perms-row { display:flex; gap:8px; flex-wrap:wrap; margin-top:6px; }
    .perm-toggle { display:flex; align-items:center; gap:6px; padding:6px 12px; border:1px solid var(--border); border-radius:8px; cursor:pointer; font-size:0.82rem; font-weight:500; color:var(--text-secondary); transition:all 0.15s; input { display:none; } &.selected { border-color:var(--accent); background:var(--accent-dim); color:var(--accent); } }
    .token-active-badge { margin-top:10px; font-size:0.82rem; color:var(--green); display:flex; align-items:center; gap:6px; }
    .created-token-box { margin-top:16px; background:var(--bg-primary); border:1px solid rgba(249,115,22,0.4); border-radius:10px; padding:16px; display:flex; flex-direction:column; gap:10px; }
    .created-token-label { font-size:0.82rem; color:var(--accent); font-weight:600; display:flex; align-items:center; gap:6px; }
    .created-token-value { font-family:var(--font-mono); font-size:0.85rem; color:var(--text-primary); word-break:break-all; }
    .loading-state { display:flex; justify-content:center; padding:60px; }
    .empty-state { padding:48px; text-align:center; color:var(--text-muted); font-size:0.9rem; }
    .auth-error { display:flex; align-items:flex-start; gap:14px; padding:24px; background:var(--red-dim); strong { display:block; color:var(--red); margin-bottom:4px; } p { font-size:0.85rem; color:var(--text-secondary); } }
    .table-wrapper { overflow-x:auto; }
    .tokens-table { width:100%; border-collapse:collapse; }
    .tokens-table th { padding:10px 16px; text-align:left; font-size:0.75rem; text-transform:uppercase; letter-spacing:0.06em; color:var(--text-muted); border-bottom:1px solid var(--border); }
    .tokens-table td { padding:10px 16px; border-bottom:1px solid var(--border); font-size:0.875rem; }
    .tokens-table tr:last-child td { border-bottom:none; }
    .tokens-table tr.expired td { opacity:0.5; }
    .name-cell { font-weight:600; color:var(--text-primary); }
    .mono { font-family:var(--font-mono); }
    .id-cell { color:var(--text-muted); font-size:0.8rem; }
    .time-cell { color:var(--text-muted); font-size:0.8rem; white-space:nowrap; }
    .perms-chips { display:flex; gap:4px; flex-wrap:wrap; }
    .perm-chip { padding:2px 8px; border-radius:20px; font-size:0.72rem; font-weight:600; }
    .perm-read   { background:var(--blue-dim,rgba(59,130,246,0.12)); color:var(--blue,#3b82f6); }
    .perm-write  { background:var(--accent-dim); color:var(--accent); }
    .perm-delete { background:var(--red-dim); color:var(--red); }
    .perm-admin  { background:rgba(168,85,247,0.12); color:#a855f7; }
  `]
})
export class AdminComponent implements OnInit {
  private store = inject(ApexStoreService);
  private toast = inject(ToastService);

  readonly allPermissions: Permission[] = ['Read', 'Write', 'Delete', 'Admin'];
  readonly RefreshIcon           = RefreshIcon;
  readonly LockPasswordIcon      = LockPasswordIcon;
  readonly Add01Icon             = Add01Icon;
  readonly Delete01Icon          = Delete01Icon;
  readonly Copy01Icon            = Copy01Icon;
  readonly CheckmarkCircle01Icon = CheckmarkCircle01Icon;
  readonly LockIcon              = LockIcon;

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

  ngOnInit(): void { this.hasSessionToken.set(!!localStorage.getItem('apex_token')); this.load(); }

  load(): void {
    this.loading.set(true); this.authError.set(false);
    this.store.listTokens().subscribe({
      next: (t) => { this.tokens.set(t); this.loading.set(false); },
      error: (e) => { if (e.status === 401 || e.status === 403) this.authError.set(true); else this.toast.error(e?.error?.message ?? 'Failed to load tokens'); this.loading.set(false); }
    });
  }

  saveSessionToken(): void {
    const t = this.sessionToken.trim(); if (!t) return;
    localStorage.setItem('apex_token', t);
    this.hasSessionToken.set(true); this.sessionToken = '';
    this.toast.success('Token salvo na sessão'); this.load();
  }

  clearSessionToken(): void {
    localStorage.removeItem('apex_token'); this.hasSessionToken.set(false); this.tokens.set([]);
    this.toast.info('Token removido');
  }

  togglePerm(p: Permission): void {
    this.newPerms.update(l => l.includes(p) ? l.filter(x => x !== p) : [...l, p]);
  }

  createToken(): void {
    const name = this.newName.trim(); if (!name || this.newPerms().length === 0) return;
    this.loadingCreate.set(true); this.createdToken.set(null);
    this.store.createToken(name, this.newPerms(), this.newExpiry ?? undefined).subscribe({
      next: (r) => { this.toast.success(`Token "${name}" criado!`); if (r.data?.token) this.createdToken.set(r.data.token); this.newName = ''; this.newExpiry = null; this.newPerms.set([]); this.load(); this.loadingCreate.set(false); },
      error: (e) => { this.toast.error(e?.error?.message ?? 'Create failed'); this.loadingCreate.set(false); }
    });
  }

  revokeToken(id: string): void {
    this.store.deleteToken(id).subscribe({
      next: () => { this.tokens.update(l => l.filter(t => t.id !== id)); this.toast.success('Token revogado'); },
      error: (e) => this.toast.error(e?.error?.message ?? 'Revoke failed')
    });
  }

  copyToken(): void {
    const t = this.createdToken(); if (!t) return;
    navigator.clipboard.writeText(t).then(() => this.toast.success('Copiado!'));
  }

  nsToDate(ns: number): Date { return new Date(ns / 1_000_000); }
  isExpired(t: ApiToken): boolean { return !!t.expires_at && t.expires_at < Date.now() * 1_000_000; }
}
