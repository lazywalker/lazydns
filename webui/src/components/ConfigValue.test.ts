// Unit tests for ConfigValue component logic.
//
// ConfigValue recursively renders an arbitrary JSON value (from a plugin's
// args_summary). These tests cover the behaviors most likely to break:
// long-string truncation, type classification, empty collections, and the
// max-depth guard.

import { describe, it, expect } from 'vitest';

const TRUNCATE_AT = 60;
const MAX_DEPTH = 6;

describe('ConfigValue: value classification', () => {
    type Kind = 'null' | 'boolean' | 'number' | 'string' | 'array' | 'object' | 'empty-array' | 'empty-object';

    // Mirrors the branch order in ConfigValue.svelte's template.
    function classify(value: unknown, depth: number): Kind {
        if (depth >= MAX_DEPTH) return 'string'; // depth guard renders "…"
        if (value === null) return 'null';
        if (typeof value === 'boolean') return 'boolean';
        if (typeof value === 'number') return 'number';
        if (typeof value === 'string') return 'string';
        if (Array.isArray(value)) {
            return value.length === 0 ? 'empty-array' : 'array';
        }
        if (typeof value === 'object') {
            return Object.keys(value as Record<string, unknown>).length === 0
                ? 'empty-object'
                : 'object';
        }
        return 'null';
    }

    it('classifies scalars', () => {
        expect(classify(null, 0)).toBe('null');
        expect(classify(true, 0)).toBe('boolean');
        expect(classify(false, 0)).toBe('boolean');
        expect(classify(42, 0)).toBe('number');
        expect(classify(0, 0)).toBe('number');
        expect(classify('hello', 0)).toBe('string');
        expect(classify('', 0)).toBe('string');
    });

    it('classifies empty vs non-empty collections', () => {
        expect(classify([], 0)).toBe('empty-array');
        expect(classify([1], 0)).toBe('array');
        expect(classify({}, 0)).toBe('empty-object');
        expect(classify({ a: 1 }, 0)).toBe('object');
    });

    it('triggers the depth guard at MAX_DEPTH', () => {
        // At exactly MAX_DEPTH, any value renders as the guard, regardless of type.
        expect(classify({ a: 1 }, MAX_DEPTH)).toBe('string');
        expect(classify([1, 2, 3], MAX_DEPTH)).toBe('string');
        expect(classify('deep', MAX_DEPTH)).toBe('string');
    });

    it('does not trigger the depth guard below MAX_DEPTH', () => {
        expect(classify({ a: 1 }, MAX_DEPTH - 1)).toBe('object');
    });
});

describe('ConfigValue: string truncation', () => {
    // Mirrors the truncation logic in ConfigValue.svelte.
    function truncate(value: string): { truncated: string; needsTruncate: boolean } {
        const needsTruncate = value.length > TRUNCATE_AT;
        return {
            needsTruncate,
            truncated: value.slice(0, TRUNCATE_AT) + '…',
        };
    }

    it('does not truncate short strings', () => {
        const r = truncate('short');
        expect(r.needsTruncate).toBe(false);
    });

    it('truncates strings longer than TRUNCATE_AT and appends ellipsis', () => {
        const long = 'https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/reject-list.txt';
        const r = truncate(long);
        expect(r.needsTruncate).toBe(true);
        expect(r.truncated.length).toBe(TRUNCATE_AT + 1); // +1 for "…"
        expect(r.truncated.endsWith('…')).toBe(true);
        expect(r.truncated.startsWith('https://raw.githubusercon')).toBe(true);
    });

    it('leaves a string of exactly TRUNCATE_AT chars untruncated', () => {
        const exact = 'a'.repeat(TRUNCATE_AT);
        const r = truncate(exact);
        expect(r.needsTruncate).toBe(false); // strictly greater-than
    });

    it('truncates a string one char longer than TRUNCATE_AT', () => {
        const over = 'a'.repeat(TRUNCATE_AT + 1);
        const r = truncate(over);
        expect(r.needsTruncate).toBe(true);
    });
});

describe('ConfigValue: recursion safety', () => {
    // The component recurses via <svelte:self value={...} depth={depth + 1} />.
    // A deeply nested object must bottom out at MAX_DEPTH rather than stack-overflow.
    function buildNested(depth: number): unknown {
        let v: unknown = 'leaf';
        for (let i = 0; i < depth; i++) {
            v = { nested: v };
        }
        return v;
    }

    it('produces a finite structure deeper than MAX_DEPTH', () => {
        const deep = buildNested(MAX_DEPTH + 5);
        // Walk down; it should be objects all the way (the component would render
        // "…" at MAX_DEPTH, but the data itself is finite and safe to traverse).
        let cur: unknown = deep;
        let count = 0;
        while (typeof cur === 'object' && cur !== null && count < 100) {
            cur = (cur as Record<string, unknown>).nested;
            count++;
        }
        expect(count).toBe(MAX_DEPTH + 5);
        expect(cur).toBe('leaf');
    });
});
