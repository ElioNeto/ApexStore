import { Injectable, inject } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable } from 'rxjs';
import { map } from 'rxjs/operators';
import { environment } from '../../environments/environment';

export interface KeyValue {
  key: string;
  value: string;
}

export interface StatsResponse {
  [key: string]: unknown;
}

interface ApiResponse<T> {
  success: boolean;
  message: string;
  data: T | null;
}

@Injectable({ providedIn: 'root' })
export class ApexStoreService {
  private http = inject(HttpClient);
  private baseUrl = environment.apiUrl;

  put(key: string, value: string): Observable<unknown> {
    return this.http.post(`${this.baseUrl}/keys`, { key, value });
  }

  get(key: string): Observable<{ value: string }> {
    return this.http.get<ApiResponse<{ key: string; value: string }>>(`${this.baseUrl}/keys/${key}`).pipe(
      map(response => ({ value: response.data?.value ?? '' }))
    );
  }

  getStats(): Observable<StatsResponse> {
    return this.http.get<StatsResponse>(`${this.baseUrl}/stats/all`);
  }
}

  getStats(): Observable<StatsResponse> {
    return this.http.get<StatsResponse>(`${this.baseUrl}/stats/all`);
  }
}
