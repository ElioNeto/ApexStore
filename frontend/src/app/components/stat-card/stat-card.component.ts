import { Component, input } from '@angular/core';

@Component({
  selector: 'app-stat-card',
  standalone: true,
  template: `
    <div class="stat-card">
      <div class="stat-icon">{{ icon() }}</div>
      <div class="stat-body">
        <div class="stat-label">{{ label() }}</div>
        <div class="stat-value">{{ value() }}</div>
        @if (sub()) {
          <div class="stat-sub">{{ sub() }}</div>
        }
      </div>
    </div>
  `,
  styles: [`
    .stat-card {
      background: var(--bg-card);
      border: 1px solid var(--border);
      border-radius: var(--radius-lg);
      padding: 20px;
      display: flex;
      align-items: flex-start;
      gap: 16px;
      transition: border-color 0.15s;
      &:hover { border-color: var(--border-light); }
    }
    .stat-icon { font-size: 1.8rem; line-height: 1; }
    .stat-body { flex: 1; }
    .stat-label { font-size: 0.78rem; font-weight: 500; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; margin-bottom: 4px; }
    .stat-value { font-size: 1.5rem; font-weight: 700; color: var(--text-primary); font-family: var(--font-mono); }
    .stat-sub { font-size: 0.8rem; color: var(--text-muted); margin-top: 4px; }
  `]
})
export class StatCardComponent {
  icon = input<string>('📦');
  label = input<string>('');
  value = input<string>('');
  sub = input<string>('');
}
