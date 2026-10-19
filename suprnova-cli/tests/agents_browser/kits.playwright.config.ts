import { defineConfig } from '@playwright/test';

if (!process.env.KIT_BASE_URL || !process.env.KIT_NAME) {
  throw new Error('Run through tests/agents_browser/kits.rs, which sets KIT_BASE_URL and KIT_NAME.');
}

export default defineConfig({
  testDir: '.',
  testMatch: 'kits.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: [['list'], ['json', { outputFile: process.env.KIT_AGENTS_REPORT ?? 'report.json' }]],
  outputDir: process.env.KIT_AGENTS_RESULTS ?? 'test-results',
  use: {
    baseURL: process.env.KIT_BASE_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: process.env.KIT_NAME,
      use: {
        browserName: 'chromium',
        // Chromium exposes WebMCP, `document.modelContext` and the DevTools
        // `WebMCP` domain, behind this testing flag.
        launchOptions: { args: ['--enable-features=WebMCPTesting'] },
      },
    },
  ],
});
