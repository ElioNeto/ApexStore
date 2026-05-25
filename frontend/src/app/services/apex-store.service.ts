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

  // ── Replication ──────────────────────────────────────────────────────────

  getReplicationTopology(): Observable<{ nodes: any[]; summary: any }> {
    return this.http
      .get<ApiResponse<{ nodes: any[]; summary: any }>>(`${this.baseUrl}/admin/replication`, this.opts())
      .pipe(map(r => r.data ?? { nodes: [], summary: null }));
  }

  promoteReplica(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/replication/${id}/promote`, {}, this.opts());
  }

  removeReplica(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/replication/${id}`, this.opts());
  }

  // ── Vector Search ────────────────────────────────────────────────────────

  listVectorIndexes(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ indexes: any[] }>>(`${this.baseUrl}/vector/indexes`, this.opts())
      .pipe(map(r => r.data?.indexes ?? []));
  }

  createVectorIndex(name: string, dimension: number, metric: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/vector/indexes`, { name, dimension, metric }, this.opts());
  }

  deleteVectorIndex(name: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/vector/indexes/${encodeURIComponent(name)}`, this.opts());
  }

  vectorSearch(index: string, query: string): Observable<any[]> {
    return this.http
      .post<ApiResponse<{ results: any[] }>>(`${this.baseUrl}/vector/indexes/${encodeURIComponent(index)}/search`, { query }, this.opts())
      .pipe(map(r => r.data?.results ?? []));
  }

  // ── Data Sync ────────────────────────────────────────────────────────────

  listSyncJobs(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ jobs: any[] }>>(`${this.baseUrl}/admin/sync`, this.opts())
      .pipe(map(r => r.data?.jobs ?? []));
  }

  createSyncJob(name: string, source: string, target: string, mode: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/sync`, { name, source, target, mode }, this.opts());
  }

  triggerSyncJob(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/sync/${id}/trigger`, {}, this.opts());
  }

  deleteSyncJob(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/sync/${id}`, this.opts());
  }

  // ── CDC ──────────────────────────────────────────────────────────────────

  getCDCConfig(): Observable<{ config: any; tables: any[] }> {
    return this.http
      .get<ApiResponse<{ config: any; tables: any[] }>>(`${this.baseUrl}/admin/cdc`, this.opts())
      .pipe(map(r => r.data ?? { config: {}, tables: [] }));
  }

  updateCDCConfig(config: any): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/cdc`, config, this.opts());
  }

  addCDCTable(table: string, events: string[]): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/cdc/tables`, { table, events }, this.opts());
  }

  removeCDCTable(table: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/cdc/tables/${encodeURIComponent(table)}`, this.opts());
  }

  // ── Bulk Import / Export ─────────────────────────────────────────────────

  listImportJobs(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ jobs: any[] }>>(`${this.baseUrl}/admin/imports`, this.opts())
      .pipe(map(r => r.data?.jobs ?? []));
  }

  createImportJob(filename: string, format: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/imports`, { filename, format }, this.opts());
  }

  deleteImportJob(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/imports/${id}`, this.opts());
  }

  exportData(format: string): Observable<Blob> {
    return this.http.get(`${this.baseUrl}/admin/export?format=${format}`, { ...this.opts(), responseType: 'blob' });
  }

  // ── Server Config ────────────────────────────────────────────────────────

  getServerConfig(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ entries: any[] }>>(`${this.baseUrl}/admin/config`, this.opts())
      .pipe(map(r => r.data?.entries ?? []));
  }

  updateServerConfig(key: string, value: string): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/config/${encodeURIComponent(key)}`, { value }, this.opts());
  }

  // ── Chaos Engineering ────────────────────────────────────────────────────

  listChaosExperiments(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ experiments: any[] }>>(`${this.baseUrl}/admin/chaos`, this.opts())
      .pipe(map(r => r.data?.experiments ?? []));
  }

  createChaosExperiment(name: string, type: string, target: string, config: Record<string, unknown>): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/chaos`, { name, type, target, config }, this.opts());
  }

  toggleChaosExperiment(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/chaos/${id}/toggle`, {}, this.opts());
  }

  deleteChaosExperiment(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/chaos/${id}`, this.opts());
  }

  // ── Telemetry ────────────────────────────────────────────────────────────

  getTelemetryConfig(): Observable<{ config: any; log_levels: any[] }> {
    return this.http
      .get<ApiResponse<{ config: any; log_levels: any[] }>>(`${this.baseUrl}/admin/telemetry`, this.opts())
      .pipe(map(r => r.data ?? { config: {}, log_levels: [] }));
  }

  updateTelemetryConfig(config: any): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/telemetry`, config, this.opts());
  }

  setLogLevel(module: string, level: string): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/telemetry/log_levels/${encodeURIComponent(module)}`, { level }, this.opts());
  }

  // ── Quotas ───────────────────────────────────────────────────────────────

  listQuotas(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ quotas: any[] }>>(`${this.baseUrl}/admin/quotas`, this.opts())
      .pipe(map(r => r.data?.quotas ?? []));
  }

  createQuota(quota: any): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/quotas`, quota, this.opts());
  }

  updateQuota(id: string, quota: any): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/quotas/${id}`, quota, this.opts());
  }

  deleteQuota(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/quotas/${id}`, this.opts());
  }

  // ── Data Scrubber ────────────────────────────────────────────────────────

  listScrubberJobs(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ jobs: any[] }>>(`${this.baseUrl}/admin/scrubber`, this.opts())
      .pipe(map(r => r.data?.jobs ?? []));
  }

  createScrubberJob(pattern: string, retention_days: number): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/scrubber`, { pattern, retention_days }, this.opts());
  }

  deleteScrubberJob(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/scrubber/${id}`, this.opts());
  }

  listIdempotencyKeys(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ keys: any[] }>>(`${this.baseUrl}/admin/idempotency`, this.opts())
      .pipe(map(r => r.data?.keys ?? []));
  }

  deleteIdempotencyKey(key: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/idempotency/${encodeURIComponent(key)}`, this.opts());
  }

  // ── Backpressure & Retry ─────────────────────────────────────────────────

  getBackpressureConfig(): Observable<any> {
    return this.http
      .get<ApiResponse<any>>(`${this.baseUrl}/admin/backpressure`, this.opts())
      .pipe(map(r => r.data ?? {}));
  }

  updateBackpressureConfig(config: any): Observable<ApiResponse<null>> {
    return this.http.put<ApiResponse<null>>(`${this.baseUrl}/admin/backpressure`, config, this.opts());
  }

  // ── WASM Plugins ─────────────────────────────────────────────────────────

  listWasmPlugins(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ plugins: any[] }>>(`${this.baseUrl}/admin/wasm`, this.opts())
      .pipe(map(r => r.data?.plugins ?? []));
  }

  uploadWasmPlugin(file: File): Observable<ApiResponse<null>> {
    const formData = new FormData();
    formData.append('file', file);
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/wasm`, formData, { headers: this.headers });
  }

  toggleWasmPlugin(id: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/wasm/${id}/toggle`, {}, this.opts());
  }

  deleteWasmPlugin(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/wasm/${id}`, this.opts());
  }

  // ── CI/CD Fixtures ───────────────────────────────────────────────────────

  listCICDFixtures(): Observable<any[]> {
    return this.http
      .get<ApiResponse<{ fixtures: any[] }>>(`${this.baseUrl}/admin/cicd`, this.opts())
      .pipe(map(r => r.data?.fixtures ?? []));
  }

  createCICDFixture(name: string, description: string, type: string): Observable<ApiResponse<null>> {
    return this.http.post<ApiResponse<null>>(`${this.baseUrl}/admin/cicd`, { name, description, type }, this.opts());
  }

  deleteCICDFixture(id: string): Observable<ApiResponse<null>> {
    return this.http.delete<ApiResponse<null>>(`${this.baseUrl}/admin/cicd/${id}`, this.opts());
  }

  generateTestData(count: number): Observable<{ records_generated: number; elapsed_ms: number }> {
    return this.http
      .post<ApiResponse<{ records_generated: number; elapsed_ms: number }>>(`${this.baseUrl}/admin/cicd/generate`, { count }, this.opts())
      .pipe(map(r => r.data ?? { records_generated: 0, elapsed_ms: 0 }));
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
