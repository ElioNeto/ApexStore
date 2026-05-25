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
  BookOpenIcon,
  NoteEditIcon,
  Share08Icon,
  Tag01Icon,
  HardDriveIcon,
  CheckmarkCircle01Icon,
  DatabaseIcon,
  Search01Icon,
  CpuIcon,
} from '@hugeicons/core-free-icons';
import { ToastComponent } from './components/toast/toast.component';
import type { IconSvgObject } from '@hugeicons/angular';

interface NavItem { path: string; icon: IconSvgObject; label: string; }
interface NavGroup { label: string; items: NavItem[]; }

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [RouterOutlet, RouterLink, RouterLinkActive, ToastComponent, HugeiconsIconComponent],
  templateUrl: './app.component.html',
  styleUrl: './app.component.scss'
})
export class AppComponent {
  readonly VERSION = '2.1.24';
  readonly DOCUMENTATION_URL = 'https://elioneto.github.io/ApexStore/';
  readonly ZapIcon = ZapIcon;
  readonly GithubIcon = Github01Icon;
  readonly BookOpenIcon = BookOpenIcon;

  navGroups = signal<NavGroup[]>([
    {
      label: 'General',
      items: [
        { path: '/dashboard', icon: Home01Icon, label: 'Dashboard' },
        { path: '/stats', icon: BarChartIcon, label: 'Statistics' },
        { path: '/health', icon: CheckmarkCircle01Icon, label: 'Health' },
        { path: '/resilience', icon: CpuIcon, label: 'Resilience' },
      ]
    },
    {
      label: 'Data',
      items: [
        { path: '/keys', icon: Key01Icon, label: 'Key Explorer' },
        { path: '/features', icon: Flag01Icon, label: 'Feature Flags' },
        { path: '/sql-runner', icon: Search01Icon, label: 'SQL Runner' },
      ]
    },
    {
      label: 'Content',
      items: [
        { path: '/notes', icon: NoteEditIcon, label: 'Notes' },
        { path: '/graph', icon: Share08Icon, label: 'Graph View' },
        { path: '/tags', icon: Tag01Icon, label: 'Tags' },
        { path: '/time-travel', icon: Share08Icon, label: 'Time Travel' },
      ]
    },
    {
      label: 'System',
      items: [
        { path: '/compaction', icon: HardDriveIcon, label: 'Compaction' },
        { path: '/rate-limits', icon: BarChartIcon, label: 'Rate Limits' },
        { path: '/backup', icon: DatabaseIcon, label: 'Backup' },
      ]
    },
    {
      label: 'Integrations',
      items: [
        { path: '/webhooks', icon: ZapIcon, label: 'Webhooks' },
        { path: '/pubsub', icon: Share08Icon, label: 'Pub/Sub' },
      ]
    },
    {
      label: 'Admin',
      items: [
        { path: '/admin', icon: LockPasswordIcon, label: 'Tokens' },
        { path: '/access-control', icon: LockPasswordIcon, label: 'Access Control' },
      ]
    },
  ]);
}
