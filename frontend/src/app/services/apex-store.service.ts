import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { Observable } from 'rxjs';
import { map } from 'rxjs/operators';
import { environment } from '../../environments/environment';

export interface KeyValue { key: string; value: string; }
export interface BatchRecord { key: string; value: string; }
export interface SearchResult { key: string; value: string; }
export interface FeatureFlag { name: string; enabled: boolean; description: string; }
export interface ApiToken {
  id: string;
  name: string;
  token?: string;
  created_at: number;
  expires_at?: number;
  permissions: Permission[];
}
export type Permission = 'Read' | 'Write' | 'Delete' | 'Admin';

interface ApiResponse<T> {
  success: boolean;
  message: string;
  data: T | null;
}

@Injectable({ providedIn: 'root' })
export class ApexStoreService {
  private http = inject(HttpClient);
  private baseUrl = environment.apiUrl;

  private get headers(): HttpHeaders {
    const token = localStorage.getItem('apex_token');
    return token ? new HttpHeaders({ Authorization: `Bearer ${token}` }) : new HttpHeaders();
  }

  private opts() { return { headers: this.headers }; }

  // ── Keys ──────────────────────────────────────────────────────────────────

  put(key: string, value: string): Observable<ApiResponse<{ key: string }>> {
    return this.http.post<ApiResponse<{ key: string }>>(`${this.baseUrl}/keys`, { key, value }, this.opts());
  }

  get(key: string): Observable<{ key: string; value: string }> {
    return this.http
      .get<ApiResponse<{ key: string; value: string }>>(`${this.baseUrl}/keys/${encodeURIComponent(key)}`, this.opts())
      .pipe(map(r => r.data!));
  }

  delete(key: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/keys/${encodeURIComponent(key)}`, this.opts());
  }

  listKeys(): Observable<string[]> {
    return this.http
      .get<ApiResponse<{ keys: string[] }>>(`${this.baseUrl}/keys`, this.opts())
      .pipe(map(r => r.data?.keys ?? []));
  }

  search(q: string, prefix = false): Observable<SearchResult[]> {
    const params = `q=${encodeURIComponent(q)}&prefix=${prefix}`;
    return this.http
      .get<ApiResponse<{ records: SearchResult[] }>>(`${this.baseUrl}/keys/search?${params}`, this.opts())
      .pipe(map(r => r.data?.records ?? []));
  }

  setBatch(records: BatchRecord[]): Observable<ApiResponse<{ count: number }>> {
    return this.http.post<ApiResponse<{ count: number }>>(`${this.baseUrl}/keys/batch`, { records }, this.opts());
  }

  scan(): Observable<SearchResult[]> {
    return this.http
      .get<ApiResponse<{ records: SearchResult[] }>>(`${this.baseUrl}/scan`, this.opts())
      .pipe(map(r => r.data?.records ?? []));
  }

  // ── Stats ─────────────────────────────────────────────────────────────────

  getHealth(): Observable<ApiResponse<null>> {
    return this.http.get<ApiResponse<null>>(`${this.baseUrl}/health`);
  }

  getStats(): Observable<Record<string, unknown>> {
    return this.http
      .get<ApiResponse<Record<string, unknown>>>(`${this.baseUrl}/stats/all`, this.opts())
      .pipe(map(r => r.data ?? {}));
  }

  // ── Features ──────────────────────────────────────────────────────────────

  listFeatures(): Observable<{ version: number; features: FeatureFlag[] }> {
    return this.http
      .get<ApiResponse<{ version: number; features: FeatureFlag[] }>>(`${this.baseUrl}/features`, this.opts())
      .pipe(map(r => r.data ?? { version: 0, features: [] }));
  }

  setFeature(name: string, enabled: boolean, description = ''): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/features/${encodeURIComponent(name)}`, { enabled, description }, this.opts());
  }

  // ── Admin / Tokens ────────────────────────────────────────────────────────

  listTokens(): Observable<ApiToken[]> {
    return this.http
      .get<ApiResponse<{ tokens: ApiToken[] }>>(`${this.baseUrl}/admin/tokens`, this.opts())
      .pipe(map(r => r.data?.tokens ?? []));
  }

  createToken(name: string, permissions: Permission[], expires_in_days?: number): Observable<ApiResponse<ApiToken>> {
    return this.http.post<ApiResponse<ApiToken>>(`${this.baseUrl}/admin/tokens`, { name, permissions, expires_in_days }, this.opts());
  }

  deleteToken(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/tokens/${id}`, this.opts());
  }
}
