// API client for LazyDNS WebUI

import type { SecurityEvent, SecurityEventType } from './types';

const API_BASE = '/api';

export interface DashboardOverviewResponse {
    status: string;
    uptime_secs: number;
    metrics: {
        total_queries: number;
        queries_per_second: number;
        cache_hit_rate: number;
        cache_hits: number;
        cache_misses: number;
        error_responses: number;
        blocked_queries: number;
        unique_domains: number;
        unique_clients: number;
    };
    recent_alerts: number;
    active_sse_connections: number;
}

export interface UpstreamHealthItem {
    address: string;
    tag: string | null;
    plugin?: string;
    status: string;
    success_rate: number;
    avg_response_time_ms: number;
    queries: number;
    successes: number;
    failures: number;
    last_success: string | null;
}

export interface UpstreamHealthResponse {
    upstreams: UpstreamHealthItem[];
}

export interface TopDomainsResponse {
    domains: Array<{
        rank: number;
        key: string;
        count: number;
    }>;
    total_unique: number;
}

export interface TopClientsResponse {
    clients: Array<{
        rank: number;
        key: string;
        count: number;
    }>;
    total_unique: number;
}

export interface QpsHistoryResponse {
    points: Array<{
        timestamp: number;  // Relative seconds since collection start
        value: number;
    }>;
    current_qps: number;
    stats: {
        min: number;
        max: number;
        avg: number;
        count: number;
    };
}

export interface LatencyResponse {
    distribution: {
        buckets: Array<{
            label: string;
            count: number;
            percentage: number;
        }>;
        total: number;
        p50_ms: number;
        p95_ms: number;
        p99_ms: number;
        max_ms: number;
        avg_ms: number;
    };
}

// --- Config dump types (mirrors backend ConfigDumpResponse) ---

export interface ConfigLogSummary {
    level: string;
    console: boolean;
    format: string;
    file_enabled: boolean;
}

export interface ConfigAddrSummary {
    enabled: boolean;
    addr: string;
}

export interface ConfigWebSummary {
    enabled: boolean;
    listen: string;
}

export interface ConfigSequenceStep {
    matches: string | null;
    exec: string | null;
}

export interface ConfigPluginSummary {
    tag: string;
    plugin_type: string;
    priority: number;
    /** Raw plugin args; frontend interprets per type. */
    args_summary: Record<string, unknown>;
    is_sequence: boolean;
    sequence_steps: ConfigSequenceStep[] | null;
}

export interface ConfigDumpResponse {
    version: string;
    log: ConfigLogSummary;
    admin: ConfigAddrSummary | null;
    monitoring: ConfigAddrSummary | null;
    web: ConfigWebSummary | null;
    plugin_count: number;
    plugins: ConfigPluginSummary[];
}

class ApiClient {
    private baseUrl: string;

    constructor(baseUrl: string = API_BASE) {
        this.baseUrl = baseUrl;
    }

    private async fetch<T>(endpoint: string): Promise<T> {
        const response = await fetch(`${this.baseUrl}${endpoint}`);
        if (!response.ok) {
            throw new Error(`API error: ${response.status} ${response.statusText}`);
        }
        return response.json();
    }

    private async post<T>(endpoint: string, body?: unknown): Promise<T> {
        const response = await fetch(`${this.baseUrl}${endpoint}`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: body ? JSON.stringify(body) : undefined,
        });
        if (!response.ok) {
            const errorBody = await response.json().catch(() => ({}));
            throw new Error(errorBody.error || `API error: ${response.status} ${response.statusText}`);
        }
        return response.json();
    }

    // Dashboard
    async getDashboardOverview(): Promise<DashboardOverviewResponse> {
        return this.fetch<DashboardOverviewResponse>('/dashboard/overview');
    }

    async getRecentAlerts(): Promise<{ alerts: Alert[]; total: number }> {
        return this.fetch<{ alerts: Alert[]; total: number }>('/alerts/recent');
    }

    // Server features
    async getServerFeatures(): Promise<{ admin: boolean; metrics: boolean; audit: boolean; web_embed: boolean }> {
        return this.fetch<{ admin: boolean; metrics: boolean; audit: boolean; web_embed: boolean }>('/features');
    }

    // Metrics
    async getUpstreamHealth(): Promise<UpstreamHealthResponse> {
        return this.fetch<UpstreamHealthResponse>('/metrics/upstream-health');
    }

    async getTopDomains(limit: number = 10, timeWindow?: string): Promise<TopDomainsResponse> {
        const params = new URLSearchParams({ limit: limit.toString() });
        if (timeWindow) params.append('window', timeWindow);
        return this.fetch<TopDomainsResponse>(`/metrics/top-domains?${params}`);
    }

    async getTopClients(limit: number = 10, timeWindow?: string): Promise<TopClientsResponse> {
        const params = new URLSearchParams({ limit: limit.toString() });
        if (timeWindow) params.append('window', timeWindow);
        return this.fetch<TopClientsResponse>(`/metrics/top-clients?${params}`);
    }

    async getQpsHistory(timeWindow?: string): Promise<QpsHistoryResponse> {
        const params = new URLSearchParams();
        if (timeWindow) params.append('window', timeWindow);
        return this.fetch<QpsHistoryResponse>(`/metrics/qps?${params}`);
    }

    async getLatencyDistribution(timeWindow?: string): Promise<LatencyResponse> {
        const params = new URLSearchParams();
        if (timeWindow) params.append('window', timeWindow);
        return this.fetch<LatencyResponse>(`/metrics/latency?${params}`);
    }

    // Admin operations
    async clearCache(): Promise<{ success: boolean; message: string }> {
        return this.post<{ success: boolean; message: string }>('/admin/cache/clear');
    }

    async reloadConfig(path?: string): Promise<{ success: boolean; message: string }> {
        return this.post<{ success: boolean; message: string }>('/admin/config/reload', { path });
    }

    async getCacheStats(): Promise<{
        size: number;
        hits: number;
        misses: number;
        evictions: number;
        expirations: number;
        hit_rate: number;
    }> {
        return this.fetch('/dashboard/cache/stats');
    }

    async getServerInfo(): Promise<{
        version: string;
        uptime_secs: number;
        rss_bytes: number;
    }> {
        return this.fetch('/dashboard/server/info');
    }

    // Config dump (read-only)
    async getConfigDump(): Promise<ConfigDumpResponse> {
        return this.fetch<ConfigDumpResponse>('/config/dump');
    }

    // Alert management
    async acknowledgeAllAlerts(): Promise<{ success: boolean; message: string }> {
        return this.post<{ success: boolean; message: string }>('/admin/alerts/acknowledge-all');
    }

    async acknowledgeAlert(alertId: string): Promise<{ success: boolean; message: string }> {
        return this.post<{ success: boolean; message: string }>(`/admin/alerts/acknowledge/${alertId}`);
    }

    async clearAlerts(): Promise<{ success: boolean; message: string }> {
        return this.post<{ success: boolean; message: string }>('/admin/alerts/clear');
    }

    // Log export
    async exportLogs(logType: string, format: string = 'json', limit: number = 1000): Promise<Blob> {
        const response = await fetch(`${this.baseUrl}/admin/logs/export`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({ log_type: logType, format, limit }),
        });
        if (!response.ok) {
            throw new Error(`Export failed: ${response.status} ${response.statusText}`);
        }
        return response.blob();
    }

    // SSE endpoints for audit logs
    createQueryLogStream(): EventSource {
        return new EventSource(`${this.baseUrl}/audit/query-logs/stream`);
    }

    createSecurityEventsStream(): EventSource {
        return new EventSource(`${this.baseUrl}/audit/security-events/stream`);
    }

    // Deprecated - use specific stream methods
    createAlertsStream(): EventSource {
        return new EventSource(`${this.baseUrl}/stream/alerts`);
    }
}

// Types for alerts
export interface Alert {
    id: string;
    rule_name: string;
    severity: 'info' | 'warning' | 'critical';
    message: string;
    timestamp: number;  // Unix seconds (first occurrence)
    last_updated: number;  // Unix seconds (last occurrence)
    occurrence_count: number;  // Number of times this alert has been triggered
    acknowledged: boolean;
    context?: Record<string, string>;
}

// Types for query log entries (from SSE)
export interface QueryLogEntry {
    timestamp: string;
    query_id: number;
    client_ip: string | null;
    protocol: string;
    qname: string;
    qtype: string;
    qclass: string;
    rcode: string | null;
    answer_count: number | null;
    response_time_ms: number | null;
    cached: boolean | null;
    upstream: string | null;
    answers: string[] | null;
}

// Re-export SecurityEvent from types for compatibility
export type { SecurityEvent, SecurityEventType };

export const api = new ApiClient();
export default api;
