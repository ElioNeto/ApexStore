import { Component, input } from '@angular/core';
import { HugeiconsComponent } from '@hugeicons/angular';
import type { IconSvgElement } from '@hugeicons/angular';

@Component({
  selector: 'app-stat-card',
  standalone: true,
  imports: [HugeiconsComponent],
  templateUrl: './stat-card.component.html',
  styleUrl: './stat-card.component.scss'
})
export class StatCardComponent {
  icon  = input.required<IconSvgElement[]>();
  label = input<string>('');
  value = input<string>('');
  sub   = input<string>('');
}
