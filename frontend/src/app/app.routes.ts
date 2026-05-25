import { Routes } from '@angular/router';
import { DashboardComponent } from './pages/dashboard/dashboard.component';
import { KeyExplorerComponent } from './pages/key-explorer/key-explorer.component';
import { StatsComponent } from './pages/stats/stats.component';
import { FeaturesComponent } from './pages/features/features.component';
import { AdminComponent } from './pages/admin/admin.component';
import { NotesComponent } from './pages/notes/notes.component';
import { GraphComponent } from './pages/graph/graph.component';
import { TagsComponent } from './pages/tags/tags.component';

export const routes: Routes = [
  { path: '', redirectTo: 'dashboard', pathMatch: 'full' },
  { path: 'dashboard', component: DashboardComponent },
  { path: 'keys', component: KeyExplorerComponent },
  { path: 'stats', component: StatsComponent },
  { path: 'features', component: FeaturesComponent },
  { path: 'admin', component: AdminComponent },
  { path: 'notes', component: NotesComponent },
  { path: 'graph', component: GraphComponent },
  { path: 'tags', component: TagsComponent },
];
