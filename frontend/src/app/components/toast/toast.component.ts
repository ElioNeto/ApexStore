import { Component, inject } from '@angular/core';
import { ToastService } from '../../services/toast.service';

@Component({
  selector: 'app-toast',
  standalone: true,
  template: `
    <div class="toast-container">
      @for (toast of toastService.toasts(); track toast.id) {
        <div
          class="toast toast-{{ toast.type }}"
          (click)="toastService.dismiss(toast.id)"
          style="cursor:pointer"
        >
          <span>
            @if (toast.type === 'success') { ✅ }
            @if (toast.type === 'error') { ❌ }
            @if (toast.type === 'info') { ℹ️ }
          </span>
          {{ toast.message }}
        </div>
      }
    </div>
  `
})
export class ToastComponent {
  toastService = inject(ToastService);
}
