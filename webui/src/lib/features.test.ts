// Unit tests for features loading

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

// Mock the api module
vi.mock('./api', () => ({
    default: {
        getServerFeatures: vi.fn()
    }
}));

import api from './api';

// Re-import features fresh for each test (resets module-level `initialized`)
let loadServerFeatures: typeof import('./features.svelte').loadServerFeatures;
let features: typeof import('./features.svelte').features;

describe('loadServerFeatures', () => {
    let errorLog: ReturnType<typeof vi.spyOn>;

    beforeEach(async () => {
        vi.clearAllMocks();
        vi.resetModules();
        // several cases exercise the failure path on purpose; the module's
        // console.error is expected output, not a test failure
        errorLog = vi.spyOn(console, 'error').mockImplementation(() => {});
        const mod = await import('./features.svelte');
        loadServerFeatures = mod.loadServerFeatures;
        features = mod.features;
    });

    afterEach(() => {
        errorLog.mockRestore();
    });

    it('should attempt to load features from API on first call', async () => {
        const mockFeatures = {
            admin: true,
            metrics: true,
            audit: false
        };

        (api.getServerFeatures as any).mockResolvedValueOnce(mockFeatures);

        await loadServerFeatures();

        expect(api.getServerFeatures).toHaveBeenCalled();
    });

    it('should handle API errors gracefully without rethrowing', async () => {
        const error = new Error('API Error');
        (api.getServerFeatures as any).mockRejectedValueOnce(error);

        // loadServerFeatures catches errors and doesn't rethrow
        await expect(loadServerFeatures()).resolves.toBeUndefined();
    });

    it('should handle all features enabled', async () => {
        const allEnabled = {
            admin: true,
            metrics: true,
            audit: true
        };

        (api.getServerFeatures as any).mockResolvedValueOnce(allEnabled);

        await loadServerFeatures();
    });

    it('should handle all features disabled', async () => {
        const allDisabled = {
            admin: false,
            metrics: false,
            audit: false
        };

        (api.getServerFeatures as any).mockResolvedValueOnce(allDisabled);

        await loadServerFeatures();
    });

    it('should handle network timeout by catching error', async () => {
        const timeoutError = new Error('Network timeout');
        (api.getServerFeatures as any).mockRejectedValueOnce(timeoutError);

        // Should not throw - error is caught
        await expect(loadServerFeatures()).resolves.toBeUndefined();
    });

    it('should handle empty response', async () => {
        (api.getServerFeatures as any).mockResolvedValueOnce({});

        await expect(loadServerFeatures()).resolves.not.toThrow();
    });

    it('should handle null response', async () => {
        (api.getServerFeatures as any).mockResolvedValueOnce(null);

        await expect(loadServerFeatures()).resolves.not.toThrow();
    });

    it('should process response from API without errors', async () => {
        const response = {
            admin: true,
            metrics: false,
            audit: true
        };

        (api.getServerFeatures as any).mockResolvedValueOnce(response);

        await expect(loadServerFeatures()).resolves.not.toThrow();
    });

    it('should handle 401 unauthorized response by catching error', async () => {
        const error = new Error('Unauthorized');
        (api.getServerFeatures as any).mockRejectedValueOnce(error);

        // Error is caught, not rethrown
        await expect(loadServerFeatures()).resolves.not.toThrow();
    });

    it('should handle 500 server error response by catching error', async () => {
        const error = new Error('Internal Server Error');
        (api.getServerFeatures as any).mockRejectedValueOnce(error);

        // Error is caught, not rethrown
        await expect(loadServerFeatures()).resolves.not.toThrow();
    });

    it('should not call API again after successful initialization', async () => {
        const mockFeatures = { admin: true, metrics: true, audit: false };
        (api.getServerFeatures as any).mockResolvedValue(mockFeatures);

        await loadServerFeatures();
        await loadServerFeatures(); // second call should skip

        // API should only be called once
        expect(api.getServerFeatures).toHaveBeenCalledTimes(1);
    });

    it('should retry after error (initialized stays false on error)', async () => {
        // First call fails
        (api.getServerFeatures as any).mockRejectedValueOnce(new Error('fail'));
        await loadServerFeatures();

        // Second call should still try (initialized is false after error)
        (api.getServerFeatures as any).mockResolvedValueOnce({ admin: true, metrics: false, audit: false });
        await loadServerFeatures();

        expect(api.getServerFeatures).toHaveBeenCalledTimes(2);
    });
});

describe('features object', () => {
    it('should have admin property', () => {
        expect(features).toHaveProperty('admin');
    });

    it('should have metrics property', () => {
        expect(features).toHaveProperty('metrics');
    });

    it('should have audit property', () => {
        expect(features).toHaveProperty('audit');
    });

    it('should initialize with boolean types', () => {
        expect(typeof features.admin).toBe('boolean');
        expect(typeof features.metrics).toBe('boolean');
        expect(typeof features.audit).toBe('boolean');
    });
});
