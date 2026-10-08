import { defineConfig } from "@playwright/test";

// The component cases need no server: each page is assembled from the
// shipped component files. The default config runs them as well, beside the
// cases that need the hosts.
export default defineConfig({
  testDir: "./e2e/components",
  fullyParallel: false,
  workers: 1,
  timeout: 20_000,
  expect: { timeout: 5_000 },
  use: {
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { browserName: "chromium" } },
    { name: "firefox", use: { browserName: "firefox" } },
    { name: "webkit", use: { browserName: "webkit" } },
  ],
});
