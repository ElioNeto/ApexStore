import { Component, signal } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive } from '@angular/router';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import {
  Home01Icon,
  BarChartIcon,
  Key01Icon,
  Flag01Icon,
  LockPasswordIcon,
  Github01Icon,
  ZapIcon,
} from '@hugeicons/core-free-icons';
import { ToastComponent } from './components/toast/toast.component';
import type { IconSvgObject } from '@hugeicons/angular';

interface NavItem { path: string; icon: IconSvgObject; label: string; }
interface NavGroup { label: string; items: NavItem[]; }

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [RouterOutlet, RouterLink, RouterLinkActive, ToastComponent, HugeiconsIconComponent],
  template: `
    <div class="app-shell">
      <aside class="sidebar">
        <div class="sidebar-logo">
          <hugeicons-icon [icon]="ZapIcon" [size]="22" [strokeWidth]="1.5" color="var(--accent)" />
          <span class="logo-text">ApexStore</span>
          <span class="badge badge-warning" style="font-size:0.65rem;">v2</span>
        </div>

        <nav class="sidebar-nav">
          @for (group of navGroups(); track group.label) {
            <div class="nav-group-label">{{ group.label }}</div>
            @for (item of group.items; track item.path) {
              <a [routerLink]="item.path" routerLinkActive="active" class="nav-item">
                <span class="nav-icon">
                  <hugeicons-icon [icon]="item.icon" [size]="17" [strokeWidth]="1.5" />
                </span>
                <span>{{ item.label }}</span>
              </a>
            }
          }
        </nav>

        <div class="sidebar-footer">
          <a href="https://github.com/ElioNeto/ApexStore" target="_blank" class="nav-item">
            <span class="nav-icon">
              <hugeicons-icon [icon]="GithubIcon" [size]="17" [strokeWidth]="1.5" />
            </span>
            <span>GitHub</span>
          </a>
        </div>
      </aside>

      <main class="main-content">
        <router-outlet />
      </main>
    </div>
    <app-toast />
  `,
  styles: [`
    .app-shell { display: flex; min-height: 100vh; }
    .sidebar {
      width: 220px; min-width: 220px;
      background: var(--bg-secondary);
      border-right: 1px solid var(--border);
      display: flex; flex-direction: column; padding: 20px 0;
    }
    .sidebar-logo {
      display: flex; align-items: center; gap: 10px;
      padding: 0 20px 24px;
      border-bottom: 1px solid var(--border); margin-bottom: 16px;
    }
    .logo-text { font-weight: 700; font-size: 1.05rem; color: var(--text-primary); }
    .sidebar-nav { display: flex; flex-direction: column; gap: 2px; padding: 0 12px; flex: 1; }
    .nav-group-label {
      font-size: 0.68rem; font-weight: 600; text-transform: uppercase;
      letter-spacing: 0.08em; color: var(--text-muted);
      padding: 14px 12px 4px;
    }
    .nav-item {
      display: flex; align-items: center; gap: 10px;
      padding: 10px 12px; border-radius: 8px;
      color: var(--text-secondary); text-decoration: none;
      font-size: 0.9rem; font-weight: 500; transition: all 0.15s;
      &:hover { background: var(--bg-card); color: var(--text-primary); }
      &.active { background: var(--accent-dim); color: var(--accent); }
    }
    .nav-icon {
      width: 20px; display: flex; align-items: center; justify-content: center;
    }
    .sidebar-footer {
      padding: 0 12px;
      border-top: 1px solid var(--border); padding-top: 12px; margin-top: 12px;
    }
    .main-content { flex: 1; overflow-y: auto; }
  `]
})
export class AppComponent {
  readonly ZapIcon = ZapIcon;
  readonly GithubIcon = Github01Icon;

  navGroups = signal<NavGroup[]>([
    {
      label: 'General',
      items: [
        { path: '/dashboard', icon: Home01Icon, label: 'Dashboard' },
        { path: '/stats', icon: BarChartIcon, label: 'Statistics' },
      ]
    },
    {
      label: 'Data',
      items: [
        { path: '/keys', icon: Key01Icon, label: 'Key Explorer' },
        { path: '/features', icon: Flag01Icon, label: 'Feature Flags' },
      ]
    },
    {
      label: 'Admin',
      items: [
        { path: '/admin', icon: LockPasswordIcon, label: 'Tokens' },
      ]
    },
  ]);
}
