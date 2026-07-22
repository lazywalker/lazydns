import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig as defineVitestConfig } from 'vitest/config'

export default defineConfig({
    plugins: [svelte()],
    server: {
        host: '0.0.0.0',
        port: 5173,
        proxy: {
            '/api': {
                target: 'http://127.0.0.1:8080',
                changeOrigin: true
            }
        }
    },
    test: {
        globals: true,
        environment: 'jsdom',
        include: ['src/**/*.{test,spec}.{js,ts}'],
        setupFiles: ['./src/test/setup.ts'],
        coverage: {
            provider: 'v8',
            reporter: ['text', 'json', 'html', 'lcov'],
            include: ['src/lib/**/*.{js,ts}'],
            exclude: [
                'node_modules/',
                'src/test/',
                'src/**/*.test.ts',
                'src/**/*.spec.ts',
                'src/lib/mock.ts',
                'src/lib/types.ts'
            ],
            all: true,
            thresholds: {
                lines: 70,
                functions: 65,
                branches: 75,
                statements: 70
            }
        }
    },
    build: {
        chunkSizeWarningLimit: 1500,
        outDir: 'dist',
        emptyOutDir: true,
        rollupOptions: {
            output: {
                manualChunks: {
                    // Separate vendor libraries into their own chunks
                    'vendor-core': ['svelte', 'svelte/animate', 'svelte/transition', 'svelte/easing'],
                    'vendor-charts': ['echarts'],
                    'vendor-router': ['svelte-spa-router']
                }
            }
        }
    }
})
