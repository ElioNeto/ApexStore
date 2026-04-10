import { Component, signal } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive } from '@angular/router';
import { ToastComponent } from './components/toast/toast.component';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [RouterOutlet, RouterLink, RouterLinkActive, ToastComponent],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss'
})
export class AppComponent {
  navGroups = signal([
    {
      label: 'General',
      items: [
        { path: '/dashboard', icon: '🏠', label: 'Dashboard' },
        { path: '/stats',     icon: '📊', label: 'Statistics' },
      ]
    },
    {
      label: 'Data',
      items: [
        { path: '/keys',     icon: '🔑', label: 'Key Explorer' },
        { path: '/features', icon: '🚩', label: 'Feature Flags' },
      ]
    },
    {
      label: 'Admin',
      items: [
        { path: '/admin', icon: '🔐', label: 'Tokens' },
      ]
    },
  ]);
}
