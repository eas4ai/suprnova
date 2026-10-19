import { defineConfig } from '@playwright/test';

if (!process.env.AGENTS_BASE_URL) {
  throw new Error('Run through agents_browser.rs, which starts the host and sets AGENTS_BASE_URL.');
}

export default defineConfig({
  testDir: '.',
  testMatch: 'agents.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  reporter: 'list',
  outputDir: process.env.AGENTS_BROWSER_RESULTS ?? 'test-results',
  use: {
    baseURL: process.env.AGENTS_BASE_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium-webmcp',
      use: {
        browserName: 'chromium',
        // Chromium exposes WebMCP, `document.modelContext` and the DevTools
        // `WebMCP` domain, behind this testing flag.
        launchOptions: { args: ['--enable-features=WebMCPTesting'] },
      },
    },
  ],
});
