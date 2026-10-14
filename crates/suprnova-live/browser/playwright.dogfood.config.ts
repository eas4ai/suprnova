import { defineConfig } from "@playwright/test";

// The dogfood form cases against the dogfood application host alone. The
// default config runs them as well, beside every other host.
export default defineConfig({
  testDir: "./e2e",
  testMatch: ["app-dogfood-forms.spec.ts"],
  fullyParallel: false,
  workers: 1,
  timeout: 20_000,
  expect: { timeout: 5_000 },
  use: {
    trace: "retain-on-failure",
  },
  webServer: [
    {
      // The host boots the framework, which reads an unset APP_ENV as production
      // and refuses to start without an APP_KEY; a test host runs in development.
      command:
        "APP_ENV=development SUPRNOVA_LIVE_DOGFOOD_PORT=4178 SESSION_SECURE=false cargo run --manifest-path ../Cargo.toml -p app --example live_dogfood_host",
      url: "http://127.0.0.1:4178/health",
      reuseExistingServer: false,
      timeout: 600_000,
    },
  ],
  projects: [
    { name: "chromium", use: { browserName: "chromium" } },
    { name: "firefox", use: { browserName: "firefox" } },
    { name: "webkit", use: { browserName: "webkit" } },
  ],
});
