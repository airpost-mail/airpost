// The browser run against a real engine: `npm run e2e`.
// Not in CI. Everything it starts lives in a fresh /tmp directory, screenshots included.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: '*.spec.ts',
  globalSetup: './global-setup.ts',
  workers: 1,
  fullyParallel: false,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  reporter: [['list']],
  outputDir: process.env.E2E_OUT ?? '/tmp/tern-web-e2e-results',
  use: {
    viewport: { width: 1400, height: 900 },
    acceptDownloads: true,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
});
