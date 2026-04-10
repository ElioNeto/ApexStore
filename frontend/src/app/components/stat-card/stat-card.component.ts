import { Component, input } from '@angular/core';
import { HugeiconsIconComponent } from '@hugeicons/angular';
import type { IconSvgObject } from '@hugeicons/angular';

@Component({
  selector: 'app-stat-card',
  standalone: true,
  imports: [HugeiconsIconComponent],
  templateUrl: './stat-card.component.html',
  styleUrl: './stat-card.component.scss'
})
export class StatCardComponent {
  icon  = input.required<IconSvgObject>();
  label = input<string>('');
  value = input<string>('');
  sub   = input<string>('');
}
