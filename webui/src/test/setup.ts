// Test setup file for Vitest
import '@testing-library/jest-dom';

// jsdom does not implement EventSource; provide a minimal mock so SSE-related
// API methods can be tested.
class MockEventSource {
    url: string;
    readyState = 0;
    onopen: ((ev: Event) => void) | null = null;
    onmessage: ((ev: MessageEvent) => void) | null = null;
    onerror: ((ev: Event) => void) | null = null;

    constructor(url: string) {
        this.url = url;
    }

    addEventListener() {}
    removeEventListener() {}
    close() {
        this.readyState = 2;
    }
}

(globalThis as any).EventSource = MockEventSource;
