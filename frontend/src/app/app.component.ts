import { Component, signal } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive } from '@angular/router';
import { ToastComponent } from './components/toast/toast.component';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [RouterOutlet, RouterLink, RouterLinkActive, ToastComponent],
  template: `
    <div class="app-shell">
      <aside class="sidebar">
        <div class="sidebar-logo">
          <span class="logo-icon">⚡</span>
          <span class="logo-text">ApexStore</span>
          <span class="badge badge-warning" style="font-size:0.65rem;">v2</span>
        </div>

        <nav class="sidebar-nav">
          @for (item of navItems(); track item.path) {
            <a
              [routerLink]="item.path"
              routerLinkActive="active"
              class="nav-item"
            >
              <span class="nav-icon">{{ item.icon }}</span>
              <span>{{ item.label }}</span>
            </a>
          }
        </nav>

        <div class="sidebar-footer">
          <a
            href="https://github.com/ElioNeto/ApexStore"
            target="_blank"
            class="nav-item"
          >
            <span class="nav-icon">🔗</span>
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
    .app-shell {
      display: flex;
      min-height: 100vh;
    }

    .sidebar {
      width: 220px;
      min-width: 220px;
      background: var(--bg-secondary);
      border-right: 1px solid var(--border);
      display: flex;
      flex-direction: column;
      padding: 20px 0;
    }

    .sidebar-logo {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 0 20px 24px;
      border-bottom: 1px solid var(--border);
      margin-bottom: 16px;
    }

    .logo-icon { font-size: 1.4rem; }
    .logo-text { font-weight: 700; font-size: 1.05rem; color: var(--text-primary); }

    .sidebar-nav {
      display: flex;
      flex-direction: column;
      gap: 2px;
      padding: 0 12px;
      flex: 1;
    }

    .nav-item {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 10px 12px;
      border-radius: 8px;
      color: var(--text-secondary);
      text-decoration: none;
      font-size: 0.9rem;
      font-weight: 500;
      transition: all 0.15s;

      &:hover { background: var(--bg-card); color: var(--text-primary); }
      &.active { background: var(--accent-dim); color: var(--accent); }
    }

    .nav-icon { font-size: 1rem; width: 20px; text-align: center; }

    .sidebar-footer {
      padding: 0 12px;
      border-top: 1px solid var(--border);
      padding-top: 12px;
      margin-top: 12px;
    }

    .main-content {
      flex: 1;
      overflow-y: auto;
    }
  `]
})
export class AppComponent {
  navItems = signal([
    { path: '/dashboard', icon: '🏠', label: 'Dashboard' },
    { path: '/keys', icon: '🔑', label: 'Key Explorer' },
    { path: '/stats', icon: '📊', label: 'Statistics' },
  ]);
}
