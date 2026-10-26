import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022', sourcemap: false },
  // Under test, Svelte components must resolve to their browser build. This must only be set in
  // test mode: setting `resolve.conditions` REPLACES Vite's defaults, and doing it for the real build
  // makes Svelte compile against its server entry, which renders a blank window at runtime.
  ...(mode === 'test' ? { resolve: { conditions: ['browser'] } } : {}),
  test: {
    // Pure-logic tests run in node; component tests opt in with `// @vitest-environment jsdom`.
    environment: 'node',
    setupFiles: ['./src/test/setup.ts'],
    css: false,
  },
}));
