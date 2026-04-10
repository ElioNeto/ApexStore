import { Component, input } from '@angular/core';

@Component({
  selector: 'app-stat-card',
  standalone: true,
  imports: [],
  templateUrl: './stat-card.component.html',
  styleUrl: './stat-card.component.scss'
})
export class StatCardComponent {
  icon = input<string>('📦');
  label = input<string>('');
  value = input<string>('');
  sub = input<string>('');
}
