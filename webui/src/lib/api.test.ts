// Unit tests for API client

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { api } from './api';

// Mock fetch
globalThis.fetch = vi.fn();

describe('ApiClient', () => {
    beforeEach(() => {
        vi.clearAllMocks();
    });

    describe('getDashboardOverview', () => {
        it('should fetch dashboard overview successfully', async () => {
            const mockData = {
                status: 'healthy',
                uptime_secs: 86400,
                metrics: {
                    total_queries: 1000,
                    queries_per_second: 10,
                    cache_hit_rate: 0.75,
                    cache_hits: 750,
                    cache_misses: 250,
                    error_responses: 10,
                    blocked_queries: 100,
                    unique_domains: 50,
                    unique_clients: 5
                },
                recent_alerts: 0,
                active_sse_connections: 2
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getDashboardOverview();
            expect(result).toEqual(mockData);
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/dashboard/overview')
            );
        });

        it('should handle fetch errors', async () => {
            (globalThis.fetch as any).mockRejectedValueOnce(new Error('Network error'));
            await expect(api.getDashboardOverview()).rejects.toThrow('Network error');
        });

        it('should handle HTTP errors', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 500,
                statusText: 'Internal Server Error'
            });
            await expect(api.getDashboardOverview()).rejects.toThrow();
        });
    });

    describe('getServerFeatures', () => {
        it('should fetch server features', async () => {
            const mockData = {
                admin: true,
                metrics: true,
                audit: false,
                web_embed: true
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getServerFeatures();
            expect(result).toEqual(mockData);
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/features')
            );
        });

        it('should handle 401 unauthorized', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 401,
                statusText: 'Unauthorized'
            });

            await expect(api.getServerFeatures()).rejects.toThrow();
        });
    });

    describe('getConfigDump', () => {
        it('should fetch config dump', async () => {
            const mockData = {
                version: '0.3.15',
                log: { level: 'info', console: true, format: 'text', file_enabled: false },
                admin: null,
                monitoring: null,
                web: null,
                plugin_count: 10,
                plugins: []
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getConfigDump();
            expect(result.version).toBe('0.3.15');
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/config/dump')
            );
        });
    });

    describe('reloadConfig', () => {
        it('should POST reload request', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ success: true, message: 'Reloaded' })
            });

            const result = await api.reloadConfig('/etc/lazydns/config.yaml');
            expect(result.success).toBe(true);
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/admin/config/reload'),
                expect.objectContaining({ method: 'POST' })
            );
        });

        it('should handle reload error', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 400,
                statusText: 'Bad Request',
                json: async () => ({ error: 'Validation failed' })
            });

            await expect(api.reloadConfig()).rejects.toThrow('Validation failed');
        });
    });

    describe('clearCache', () => {
        it('should POST clear cache', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ success: true, message: 'Cache cleared' })
            });

            const result = await api.clearCache();
            expect(result.success).toBe(true);
        });
    });

    describe('alert management', () => {
        it('should acknowledge all alerts', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ success: true, message: 'Done' })
            });

            const result = await api.acknowledgeAllAlerts();
            expect(result.success).toBe(true);
        });

        it('should acknowledge a single alert by id', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ success: true, message: 'Done' })
            });

            const result = await api.acknowledgeAlert('alert-123');
            expect(result.success).toBe(true);
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/admin/alerts/acknowledge/alert-123'),
                expect.objectContaining({ method: 'POST' })
            );
        });

        it('should clear all alerts', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ success: true, message: 'Cleared' })
            });

            const result = await api.clearAlerts();
            expect(result.success).toBe(true);
        });
    });

    describe('exportLogs', () => {
        it('should download blob', async () => {
            const mockBlob = new Blob(['test,data'], { type: 'text/csv' });
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                blob: async () => mockBlob
            });

            const result = await api.exportLogs('query', 'csv', 100);
            expect(result).toBeInstanceOf(Blob);
            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('/admin/logs/export'),
                expect.objectContaining({
                    method: 'POST',
                    body: JSON.stringify({ log_type: 'query', format: 'csv', limit: 100 })
                })
            );
        });

        it('should handle export error', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 500,
                statusText: 'Internal Server Error'
            });

            await expect(api.exportLogs('query', 'csv')).rejects.toThrow();
        });
    });

    describe('SSE streams', () => {
        it('should create query log stream', () => {
            const stream = api.createQueryLogStream();
            expect(stream).toBeInstanceOf(EventSource);
            expect(stream.url).toContain('/audit/query-logs/stream');
            stream.close();
        });

        it('should create security events stream', () => {
            const stream = api.createSecurityEventsStream();
            expect(stream).toBeInstanceOf(EventSource);
            expect(stream.url).toContain('/audit/security-events/stream');
            stream.close();
        });

        it('should create alerts stream (deprecated)', () => {
            const stream = api.createAlertsStream();
            expect(stream).toBeInstanceOf(EventSource);
            expect(stream.url).toContain('/stream/alerts');
            stream.close();
        });
    });

    describe('cache stats and server info', () => {
        it('should fetch cache stats', async () => {
            const mockData = { size: 100, hits: 50, misses: 50, evictions: 1, expirations: 2, hit_rate: 50 };
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getCacheStats();
            expect(result.size).toBe(100);
            expect(result.hit_rate).toBe(50);
        });

        it('should fetch server info', async () => {
            const mockData = { version: '0.3.15', uptime_secs: 3600, rss_bytes: 1048576 };
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getServerInfo();
            expect(result.version).toBe('0.3.15');
            expect(result.uptime_secs).toBe(3600);
        });
    });

    describe('getRecentAlerts', () => {
        it('should fetch recent alerts', async () => {
            const mockData = {
                alerts: [],
                total: 0
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getRecentAlerts();
            expect(result).toEqual(mockData);
        });
    });

    describe('getUpstreamHealth', () => {
        it('should fetch upstream health', async () => {
            const mockData = {
                upstreams: [
                    {
                        address: '8.8.8.8',
                        tag: null,
                        plugin: 'http',
                        status: 'healthy',
                        success_rate: 0.99,
                        avg_response_time_ms: 10,
                        queries: 1000,
                        successes: 990,
                        failures: 10,
                        last_success: '2024-01-01T12:00:00Z'
                    }
                ]
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getUpstreamHealth();
            expect(result).toEqual(mockData);
            expect(result.upstreams[0].address).toBe('8.8.8.8');
            expect(result.upstreams[0].status).toBe('healthy');
        });
    });

    describe('getTopDomains', () => {
        it('should fetch top domains with default limit', async () => {
            const mockData = {
                domains: [
                    { rank: 1, key: 'example.com', count: 100 }
                ],
                total_unique: 50
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getTopDomains();
            expect(result.domains).toHaveLength(1);
            expect(result.total_unique).toBe(50);
        });

        it('should support custom limit parameter', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ domains: [], total_unique: 0 })
            });

            await api.getTopDomains(25);

            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('limit=25')
            );
        });

        it('should support time window parameter', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ domains: [], total_unique: 0 })
            });

            await api.getTopDomains(10, '24h');

            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('window=24h')
            );
        });
    });

    describe('getTopClients', () => {
        it('should fetch top clients with default limit', async () => {
            const mockData = {
                clients: [
                    { rank: 1, key: '192.168.1.1', count: 150 }
                ],
                total_unique: 10
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getTopClients();
            expect(result.clients).toHaveLength(1);
        });

        it('should support custom limit parameter', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ clients: [], total_unique: 0 })
            });

            await api.getTopClients(20);

            expect(globalThis.fetch).toHaveBeenCalledWith(
                expect.stringContaining('limit=20')
            );
        });
    });

    describe('getCacheStats', () => {
        it('should fetch cache statistics', async () => {
            const mockData = {
                size: 7500,
                hits: 5000,
                misses: 1250,
                evictions: 100,
                expirations: 50,
                hit_rate: 0.80
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getCacheStats();
            expect(result.size).toBe(7500);
            expect(result.hit_rate).toBe(0.80);
        });
    });

    describe('getServerInfo', () => {
        it('should fetch server information', async () => {
            const mockData = {
                version: '1.0.0',
                build_time: '2024-01-01',
                git_commit: 'abc123'
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getServerInfo();
            expect(result.version).toBe('1.0.0');
        });
    });

    describe('error handling', () => {
        it('should throw on 404 responses', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 404,
                statusText: 'Not Found'
            });

            await expect(api.getDashboardOverview()).rejects.toThrow();
        });

        it('should throw on 500 responses', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: false,
                status: 500,
                statusText: 'Internal Server Error'
            });

            await expect(api.getServerFeatures()).rejects.toThrow();
        });

        it('should throw on network timeout', async () => {
            const error = new Error('Timeout');
            (globalThis.fetch as any).mockRejectedValueOnce(error);

            await expect(api.getDashboardOverview()).rejects.toThrow('Timeout');
        });
    });

    describe('response data integrity', () => {
        it('should preserve numeric types', async () => {
            const mockData = {
                status: 'healthy',
                uptime_secs: 86400,
                metrics: {
                    total_queries: 1000,
                    queries_per_second: 10.5,
                    cache_hit_rate: 0.75,
                    cache_hits: 750,
                    cache_misses: 250,
                    error_responses: 10,
                    blocked_queries: 100,
                    unique_domains: 50,
                    unique_clients: 5
                },
                recent_alerts: 0,
                active_sse_connections: 2
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getDashboardOverview() as any;
            expect(typeof result.uptime_secs).toBe('number');
            expect(typeof result.metrics.cache_hit_rate).toBe('number');
            expect(result.uptime_secs).toBe(86400);
        });

        it('should preserve null values', async () => {
            const mockData = {
                upstreams: [
                    {
                        address: '8.8.8.8',
                        tag: null,
                        status: 'healthy',
                        success_rate: 0.99,
                        avg_response_time_ms: 10,
                        queries: 1000,
                        successes: 990,
                        failures: 10,
                        last_success: null
                    }
                ]
            };

            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => mockData
            });

            const result = await api.getUpstreamHealth();
            expect(result.upstreams[0].tag).toBe(null);
            expect(result.upstreams[0].last_success).toBe(null);
        });

        it('should handle empty arrays', async () => {
            (globalThis.fetch as any).mockResolvedValueOnce({
                ok: true,
                json: async () => ({ domains: [], total_unique: 0 })
            });

            const result = await api.getTopDomains();
            expect(Array.isArray(result.domains)).toBe(true);
            expect(result.domains.length).toBe(0);
        });
    });
});
