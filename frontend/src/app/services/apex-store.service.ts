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

export interface Note {
  path: string;
  content: string;
  updated_at?: number;
}

export interface GraphNode {
  id: string;
  label: string;
  type?: string;
}

export interface GraphEdge {
  source: string;
  target: string;
  label?: string;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
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

  // ── Notes ─────────────────────────────────────────────────────────────────

  getNotes(prefix?: string): Observable<Note[]> {
    const params = prefix ? `?prefix=${encodeURIComponent(prefix)}` : '';
    return this.http
      .get<ApiResponse<{ notes: Note[] }>>(`${this.baseUrl}/notes${params}`, this.opts())
      .pipe(map(r => r.data?.notes ?? []));
  }

  getNote(path: string): Observable<Note> {
    return this.http
      .get<ApiResponse<Note>>(`${this.baseUrl}/notes/${encodeURIComponent(path)}`, this.opts())
      .pipe(map(r => r.data!));
  }

  putNote(path: string, content: string): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/notes/${encodeURIComponent(path)}`, { content }, this.opts());
  }

  deleteNote(path: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/notes/${encodeURIComponent(path)}`, this.opts());
  }

  // ── Notes Graph ───────────────────────────────────────────────────────────

  getGraphData(path: string, depth: number = 1): Observable<GraphData> {
    return this.http
      .get<ApiResponse<GraphData>>(`${this.baseUrl}/notes/${encodeURIComponent(path)}/graph?depth=${depth}`, this.opts())
      .pipe(map(r => r.data ?? { nodes: [], edges: [] }));
  }

  // ── Tags ──────────────────────────────────────────────────────────────────

  getTags(): Observable<string[]> {
    return this.http
      .get<ApiResponse<{ tags: string[] }>>(`${this.baseUrl}/tags`, this.opts())
      .pipe(map(r => r.data?.tags ?? []));
  }

  getTagNotes(tag: string): Observable<Note[]> {
    return this.http
      .get<ApiResponse<{ notes: Note[] }>>(`${this.baseUrl}/tags/${encodeURIComponent(tag)}/notes`, this.opts())
      .pipe(map(r => r.data?.notes ?? []));
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

  // ── Compaction ──────────────────────────────────────────────────────────

  flush(): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/flush`, {}, this.opts());
  }

  compact(): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/compact`, {}, this.opts());
  }

  // ── Rate Limits ─────────────────────────────────────────────────────────

  getRateLimits(): Observable<Record<string, unknown>> {
    return this.http
      .get<ApiResponse<Record<string, unknown>>>(`${this.baseUrl}/admin/rate_limits`, this.opts())
      .pipe(map(r => r.data ?? {}));
  }

  // ── Backups ──────────────────────────────────────────────────────────────

  listBackups(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ backups: any[] }>>(`${this.baseUrl}/admin/backups`, this.opts())
      .pipe(map(r => r.data?.backups ?? []));
  }

  createBackup(name: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/backups`, { name }, this.opts());
  }

  restoreBackup(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/backups/${id}/restore`, {}, this.opts());
  }

  deleteBackup(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/backups/${id}`, this.opts());
  }

  // ── Snapshots (Time Travel) ─────────────────────────────────────────────

  listSnapshots(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ snapshots: any[] }>>(`${this.baseUrl}/notes/snapshots`, this.opts())
      .pipe(map(r => r.data?.snapshots ?? []));
  }

  createSnapshot(name: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/notes/snapshots`, { name }, this.opts());
  }

  getSnapshotNotes(id: string): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ notes: any[] }>>(`${this.baseUrl}/notes/snapshots/${id}`, this.opts())
      .pipe(map(r => r.data?.notes ?? []));
  }

  restoreSnapshot(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/notes/snapshots/${id}/restore`, {}, this.opts());
  }

  deleteSnapshot(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/notes/snapshots/${id}`, this.opts());
  }

  // ── Webhooks ────────────────────────────────────────────────────────────

  listWebhooks(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ webhooks: any[] }>>(`${this.baseUrl}/admin/webhooks`, this.opts())
      .pipe(map(r => r.data?.webhooks ?? []));
  }

  createWebhook(url: string, events: string[]): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/webhooks`, { url, events }, this.opts());
  }

  deleteWebhook(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/webhooks/${id}`, this.opts());
  }

  testWebhook(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/webhooks/${id}/test`, {}, this.opts());
  }

  // ── Pub/Sub ─────────────────────────────────────────────────────────────

  listTopics(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ topics: any[] }>>(`${this.baseUrl}/pubsub/topics`, this.opts())
      .pipe(map(r => r.data?.topics ?? []));
  }

  publishMessage(topic: string, message: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/pubsub/topics/${encodeURIComponent(topic)}`, { message }, this.opts());
  }

  listSubscriptions(topic: string): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ subscriptions: any[] }>>(`${this.baseUrl}/pubsub/topics/${encodeURIComponent(topic)}/subscriptions`, this.opts())
      .pipe(map(r => r.data?.subscriptions ?? []));
  }

  // ── SQL Runner ──────────────────────────────────────────────────────────

  executeQuery(query: string): Observable<any> {
    return this.http
      .post<ApiResponse<any>>(`${this.baseUrl}/query`, { query }, this.opts())
      .pipe(map(r => r.data ?? { columns: [], rows: [], row_count: 0, elapsed_ms: 0 }));
  }

  // ── Resilience / Circuit Breakers ───────────────────────────────────────

  getCircuitBreakers(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ circuit_breakers: any[] }>>(`${this.baseUrl}/admin/circuit_breakers`, this.opts())
      .pipe(map(r => r.data?.circuit_breakers ?? []));
  }

  // ── Access Control / Policies ───────────────────────────────────────────

  listPolicies(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ policies: any[] }>>(`${this.baseUrl}/admin/policies`, this.opts())
      .pipe(map(r => r.data?.policies ?? []));
  }

  createPolicy(name: string, resource: string, actions: string[], effect: string, priority: number): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/policies`, { name, resource, actions, effect, priority }, this.opts());
  }

  deletePolicy(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/policies/${id}`, this.opts());
  }
}
