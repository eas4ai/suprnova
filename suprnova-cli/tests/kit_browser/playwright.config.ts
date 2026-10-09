import { defineConfig } from '@playwright/test';

if (!process.env.KIT_BASE_URL || !process.env.KIT_NAME) {
  throw new Error('Run through kit_browser.rs, which sets KIT_BASE_URL and KIT_NAME.');
}

export default defineConfig({
  testDir: '.',
  testMatch: 'kits.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 10_000 },
  reporter: 'list',
  outputDir: process.env.KIT_BROWSER_RESULTS ?? 'test-results',
  use: {
    baseURL: process.env.KIT_BASE_URL,
    // A short viewport leaves even the compact dashboards below the fold.
    viewport: { width: 1280, height: 360 },
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: process.env.KIT_NAME, use: { browserName: 'chromium' } }],
});
