import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  timeout: 20_000,
  expect: { timeout: 5_000 },
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
  },
  webServer: [
    {
      command: "node test-host/server.mjs",
      url: "http://127.0.0.1:4173/health",
      reuseExistingServer: false,
      timeout: 30_000,
    },
    {
      command:
        "cargo run --manifest-path ../Cargo.toml -p suprnova-live-test-support --bin async-reference-host",
      url: "http://127.0.0.1:4174/health",
      reuseExistingServer: false,
      timeout: 60_000,
    },
    {
      command:
        "SUPRNOVA_LIVE_REFERENCE_PORT=4175 cargo run --manifest-path ../Cargo.toml -p suprnova-live-test-support --bin suprnova-live-reference-host",
      url: "http://127.0.0.1:4175/suprnova-live.assets.json",
      reuseExistingServer: false,
      timeout: 60_000,
    },
  ],
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
    { name: "firefox", use: { browserName: "firefox" } },
    { name: "webkit", use: { browserName: "webkit" } },
    {
      name: "chrome-bfcache",
      testMatch: "async-lifecycle.spec.ts",
      use: {
        browserName: "chromium",
        channel: "chrome",
        launchOptions: { ignoreDefaultArgs: ["--disable-back-forward-cache"] },
      },
    },
  ],
});
