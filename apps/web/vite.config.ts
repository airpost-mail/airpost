import path from 'node:path';
import tailwindcss from '@tailwindcss/vite';
import { tanstackRouter } from '@tanstack/router-plugin/vite';
import react from '@vitejs/plugin-react';
import { searchForWorkspaceRoot } from 'vite';
import { configDefaults, defineConfig } from 'vitest/config';

// `npm run dev` with VITE_TERN_ENGINE=gateway talks to a local tern-gateway through this
// proxy, so the page and the gateway share an origin as in production. Host and
// Origin stay localhost:5173 (no changeOrigin): start the gateway with
// `--allow-origin http://localhost:5173`.
const gateway = process.env.TERN_GATEWAY_URL ?? 'http://127.0.0.1:5180';

export default defineConfig({
  plugins: [tanstackRouter({ target: 'react', autoCodeSplitting: true }), react(), tailwindcss()],
  resolve: {
    alias: { '@': path.resolve(import.meta.dirname, './src') },
  },
  server: {
    // `@tern/client` is a link to packages/client, outside this app.
    fs: { allow: [searchForWorkspaceRoot(import.meta.dirname), '../../packages/client'] },
    proxy: {
      '/gateway': { target: gateway, ws: true, changeOrigin: false },
    },
  },
  build: {
    // Served as static files by tern-gateway; keep asset paths relative to root.
    outDir: 'dist',
    sourcemap: true,
    // The desktop's build (`build:desktop`) ships .vite/license.md for its licence page.
    license: !!process.env.TERN_WEB_LICENSES,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    css: false,
    // e2e/ is the Playwright run against a real engine (npm run e2e).
    exclude: [...configDefaults.exclude, 'e2e/**'],
  },
});
