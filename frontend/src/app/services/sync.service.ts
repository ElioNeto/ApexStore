import { Injectable, NgZone, signal } from '@angular/core';
import { Observable, Subject, filter, map } from 'rxjs';
import { environment } from '../../environments/environment';

export interface SyncChange {
  key: string;
  value: string;
  timestamp: number;
  deviceId: string;
}

export type SyncStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

@Injectable({ providedIn: 'root' })
export class SyncService {
  private ws: WebSocket | null = null;
  private baseUrl = environment.apiUrl.replace('http', 'ws');
  private messageSubject = new Subject<SyncChange>();
  private statusSubject = new Subject<SyncStatus>();
  private pingInterval: ReturnType<typeof setInterval> | null = null;
  private deviceId = '';

  readonly status = signal<SyncStatus>('disconnected');
  readonly pendingChanges = signal<number>(0);
  readonly lastSyncTimestamp = signal<number | null>(null);
  readonly lastError = signal<string | null>(null);

  constructor(private ngZone: NgZone) {}

  get onMessage(): Observable<SyncChange> {
    return this.messageSubject.asObservable();
  }

  get onStatusChange(): Observable<SyncStatus> {
    return this.statusSubject.asObservable();
  }

  connect(deviceId: string): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      return; // already connected
    }
    this.deviceId = deviceId;
    this.setStatus('connecting');

    try {
      const wsUrl = `${this.baseUrl}/ws/sync?device_id=${encodeURIComponent(deviceId)}`;
      this.ws = new WebSocket(wsUrl);

      this.ws.onopen = () => {
        this.ngZone.run(() => {
          this.setStatus('connected');
          this.lastError.set(null);
          this.startPing();
        });
      };

      this.ws.onmessage = (event: MessageEvent) => {
        this.ngZone.run(() => {
          try {
            const msg = JSON.parse(event.data);
            if (msg.type === 'sync_push' && msg.changes) {
              for (const change of msg.changes) {
                this.messageSubject.next({
                  key: change.key,
                  value: change.value,
                  timestamp: change.timestamp,
                  deviceId: change.device_id || 'server',
                });
              }
            }
            if (msg.type === 'sync_ack') {
              this.lastSyncTimestamp.set(msg.ack_timestamp);
            }
          } catch {
            // ignore malformed messages
          }
        });
      };

      this.ws.onclose = () => {
        this.ngZone.run(() => {
          this.stopPing();
          this.setStatus('disconnected');
          this.ws = null;
        });
      };

      this.ws.onerror = () => {
        this.ngZone.run(() => {
          this.lastError.set('WebSocket connection error');
          this.setStatus('error');
        });
      };
    } catch {
      this.ngZone.run(() => {
        this.lastError.set('Failed to create WebSocket connection');
        this.setStatus('error');
      });
    }
  }

  disconnect(): void {
    this.stopPing();
    if (this.ws) {
      this.ws.onclose = null; // prevent reconnect logic
      this.ws.close(1000, 'Client disconnect');
      this.ws = null;
    }
    this.setStatus('disconnected');
    this.lastSyncTimestamp.set(null);
    this.pendingChanges.set(0);
  }

  pushChange(key: string, value: string): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      this.lastError.set('Cannot push change: not connected');
      return;
    }

    const change = {
      type: 'sync_push',
      changes: [
        {
          key,
          value,
          timestamp: Date.now(),
          device_id: this.deviceId,
        },
      ],
      last_ack: this.lastSyncTimestamp() ?? 0,
    };

    try {
      this.ws.send(JSON.stringify(change));
      this.pendingChanges.update(n => n + 1);
    } catch {
      this.lastError.set('Failed to send change');
    }
  }

  private setStatus(s: SyncStatus): void {
    this.status.set(s);
    this.statusSubject.next(s);
  }

  private startPing(): void {
    this.stopPing();
    this.pingInterval = setInterval(() => {
      if (this.ws && this.ws.readyState === WebSocket.OPEN) {
        try {
          this.ws.send(JSON.stringify({ type: 'ping' }));
        } catch {
          // ignore ping failures
        }
      }
    }, 30000);
  }

  private stopPing(): void {
    if (this.pingInterval) {
      clearInterval(this.pingInterval);
      this.pingInterval = null;
    }
  }
}
