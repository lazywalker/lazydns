// Unit tests for stores

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import type { Alert } from '../lib/types';
import {
    darkMode,
    isLiveMode,
    alerts,
    unacknowledgedAlerts,
    queryLogs,
    securityEvents,
    selectedTimeWindow,
    topNavCollapsed,
    sidebarCollapsed,
    notifications,
    startLiveUpdates,
    stopLiveUpdates
} from '../lib/stores';

describe('darkMode store', () => {
    it('should default to true (dark mode enabled)', () => {
        expect(get(darkMode)).toBe(true);
    });

    it('should toggle dark mode', () => {
        darkMode.set(false);
        expect(get(darkMode)).toBe(false);
        darkMode.set(true);
        expect(get(darkMode)).toBe(true);
    });
});

describe('isLiveMode store', () => {
    it('should default to true', () => {
        expect(get(isLiveMode)).toBe(true);
    });

    it('should toggle live mode', () => {
        isLiveMode.set(false);
        expect(get(isLiveMode)).toBe(false);
    });
});

describe('alerts store', () => {
    beforeEach(() => {
        alerts.set([]);
    });

    it('should start empty', () => {
        expect(get(alerts)).toEqual([]);
    });

    it('should add alerts', () => {
        const alert: Alert = {
            id: '1',
            rule_name: 'test_rule',
            severity: 'warning',
            message: 'Test alert',
            timestamp: Math.floor(Date.now() / 1000),
            last_updated: Math.floor(Date.now() / 1000),
            occurrence_count: 1,
            acknowledged: false
        };
        alerts.set([alert]);
        expect(get(alerts)).toHaveLength(1);
    });
});

describe('unacknowledgedAlerts derived store', () => {
    beforeEach(() => {
        alerts.set([]);
    });

    it('should count unacknowledged alerts', () => {
        const now = Math.floor(Date.now() / 1000);
        alerts.set([
            { id: '1', rule_name: 'rule1', severity: 'warning', message: 'Test 1', timestamp: now, last_updated: now, occurrence_count: 1, acknowledged: false },
            { id: '2', rule_name: 'rule2', severity: 'critical', message: 'Test 2', timestamp: now, last_updated: now, occurrence_count: 1, acknowledged: true },
            { id: '3', rule_name: 'rule3', severity: 'info', message: 'Test 3', timestamp: now, last_updated: now, occurrence_count: 1, acknowledged: false }
        ]);
        expect(get(unacknowledgedAlerts)).toBe(2);
    });

    it('should return 0 when all alerts are acknowledged', () => {
        const now = Math.floor(Date.now() / 1000);
        alerts.set([
            { id: '1', rule_name: 'rule1', severity: 'warning', message: 'Test 1', timestamp: now, last_updated: now, occurrence_count: 1, acknowledged: true },
            { id: '2', rule_name: 'rule2', severity: 'critical', message: 'Test 2', timestamp: now, last_updated: now, occurrence_count: 1, acknowledged: true }
        ]);
        expect(get(unacknowledgedAlerts)).toBe(0);
    });
});

describe('queryLogs store', () => {
    beforeEach(() => {
        queryLogs.set([]);
    });

    it('should start empty', () => {
        expect(get(queryLogs)).toEqual([]);
    });

    it('should add query logs', () => {
        queryLogs.set([
            {
                timestamp: new Date().toISOString(),
                query_id: 1,
                client_ip: '192.168.1.1',
                protocol: 'udp',
                qname: 'example.com',
                qtype: 'A',
                qclass: 'IN',
                rcode: 'NOERROR',
                answer_count: 1,
                response_time_ms: 10,
                cached: false,
                upstream: null,
                answers: null
            }
        ]);
        expect(get(queryLogs)).toHaveLength(1);
    });
});

describe('securityEvents store', () => {
    beforeEach(() => {
        securityEvents.set([]);
    });

    it('should start empty', () => {
        expect(get(securityEvents)).toEqual([]);
    });
});

describe('selectedTimeWindow store', () => {
    it('should default to 1h', () => {
        expect(get(selectedTimeWindow)).toBe('1h');
    });

    it('should accept valid time windows', () => {
        selectedTimeWindow.set('1m');
        expect(get(selectedTimeWindow)).toBe('1m');

        selectedTimeWindow.set('5m');
        expect(get(selectedTimeWindow)).toBe('5m');

        selectedTimeWindow.set('24h');
        expect(get(selectedTimeWindow)).toBe('24h');

        // Reset to default
        selectedTimeWindow.set('1h');
    });
});

describe('navigation stores', () => {
    it('topNavCollapsed should default to false', () => {
        expect(get(topNavCollapsed)).toBe(false);
    });

    it('sidebarCollapsed should default to true', () => {
        expect(get(sidebarCollapsed)).toBe(true);
    });
});

describe('notifications store', () => {
    it('should add notifications', () => {
        const id = notifications.add({
            type: 'success',
            message: 'Test notification',
            duration: 0 // Don't auto-remove for testing
        });
        expect(id).toBeDefined();
        expect(get(notifications).some(n => n.id === id)).toBe(true);
    });

    it('should remove notifications', () => {
        const id = notifications.add({
            type: 'error',
            message: 'Test error',
            duration: 0
        });
        notifications.remove(id);
        expect(get(notifications).some(n => n.id === id)).toBe(false);
    });

    it('should clear all notifications', () => {
        notifications.add({ type: 'info', message: 'Test 1', duration: 0 });
        notifications.add({ type: 'info', message: 'Test 2', duration: 0 });
        notifications.clear();
        expect(get(notifications)).toEqual([]);
    });

    it('should auto-remove notification after duration', async () => {
        vi.useFakeTimers();
        notifications.add({ type: 'success', message: 'Auto-remove test', duration: 100 });
        expect(get(notifications).length).toBeGreaterThan(0);

        vi.advanceTimersByTime(150);
        expect(get(notifications).some(n => n.message === 'Auto-remove test')).toBe(false);
        vi.useRealTimers();
    });
});

describe('live update no-op functions', () => {
    it('startLiveUpdates should be callable without error', () => {
        expect(() => startLiveUpdates()).not.toThrow();
    });

    it('stopLiveUpdates should be callable without error', () => {
        expect(() => stopLiveUpdates()).not.toThrow();
    });
});
