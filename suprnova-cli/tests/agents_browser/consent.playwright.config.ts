import { defineConfig } from '@playwright/test';

if (!process.env.CONSENT_BASE_URL) {
  throw new Error('Run through tests/agents_browser/consent.rs, which starts the host and sets CONSENT_BASE_URL.');
}

export default defineConfig({
  testDir: '.',
  testMatch: 'consent.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: 'list',
  outputDir: process.env.CONSENT_BROWSER_RESULTS ?? 'test-results',
  use: {
    baseURL: process.env.CONSENT_BASE_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'chromium', use: { browserName: 'chromium' } }],
});
