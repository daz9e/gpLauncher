import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, fs: { allow: ['..'] }, watch: { ignored: ['**/src-tauri/**'] } },
  build: { target: 'safari15', outDir: 'dist', emptyOutDir: true },
  resolve: mode === 'test' ? { conditions: ['browser'] } : undefined,
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.ts'],
  },
}));
