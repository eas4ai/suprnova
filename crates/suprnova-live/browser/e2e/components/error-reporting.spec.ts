import { readFile } from "node:fs/promises";
import { expect, test, type Page } from "@playwright/test";
import { SERVER_DEFAULT_LIMITS } from "../../src/limits.js";

async function reportingPage(page: Page, enabled: boolean, failStartup = false) {
  const envelopeBodies: string[] = [];
  const limits = Object.fromEntries(
    Object.entries(SERVER_DEFAULT_LIMITS).map(([key, value]) => [
      key.replace(/[A-Z]/gu, (letter) => `_${letter.toLowerCase()}`),
      value,
    ]),
  );
  const config = {
    ...limits,
    asset_identity: "report-test",
    credentials: "same-origin",
    endpoint: "/__live/action",
    max_parallel_per_island: 1,
    max_queued_per_island: 16,
    protocol: { minimum: 1, maximum: 2 },
    request_timeout_ms: 15000,
    runtime_contract_version: 1,
    ...(enabled ? { reporting_dsn: "https://public@tracker.test/42" } : {}),
  };
  const runtime = await readFile(
    process.env["REPORTING_TEST_RUNTIME"] ??
      new URL("../../dist/suprnova-live.esm.js", import.meta.url),
    "utf8",
  );
  await page.route("https://tracker.test/**", async (route) => {
    envelopeBodies.push(route.request().postData() ?? "");
    await route.fulfill({
      status: 200,
      headers: { "Access-Control-Allow-Origin": "https://app.test" },
      body: "{}",
    });
  });
  await page.route("https://app.test/runtime.js", (route) =>
    route.fulfill({ contentType: "text/javascript", body: runtime }),
  );
  await page.route("https://app.test/", (route) =>
    route.fulfill({
      contentType: "text/html",
      headers: {
        "Content-Security-Policy":
          "default-src 'self'; script-src 'self' 'nonce-browser'; connect-src 'self' https://tracker.test",
      },
      body: `<script id="suprnova-live-config" type="application/json">${JSON.stringify(config)}</script><div data-suprnova-live-island>Invalid hydration data</div><script type="module" nonce="browser">import { boot } from './runtime.js'; try { window.reportingRuntime = boot(${failStartup ? "{effects:[{}]}" : ""}); } catch (error) { window.reportingStartupError = error.message; }</script>`,
    }),
  );
  await page.goto("https://app.test/");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          Reflect.get(window, "reportingRuntime") !== undefined ||
          Reflect.get(window, "reportingStartupError") !== undefined,
      ),
    )
    .toBe(true);
  return envelopeBodies;
}

test("Live sends hydration, uncaught errors and rejections under an origin-limited CSP", async ({
  page,
}) => {
  const bodies = await reportingPage(page, true);
  await expect.poll(() => bodies.length).toBe(1);
  await page.evaluate(() => {
    setTimeout(() => {
      throw new Error("browser uncaught regression");
    }, 0);
    void Promise.reject(new Error("browser rejection regression"));
  });
  await expect.poll(() => bodies.length).toBe(3);
  const payload = bodies.join("\n");
  expect(payload).toContain("island_invalid");
  expect(payload).toContain("browser uncaught regression");
  expect(payload).toContain("browser rejection regression");
  await page.evaluate(() => {
    const runtime = Reflect.get(window, "reportingRuntime") as { stop: () => void };
    runtime.stop();
  });
  await page.evaluate(() => {
    setTimeout(() => {
      throw new Error("after stop");
    }, 0);
  });
  await page.waitForTimeout(50);
  expect(bodies).toHaveLength(3);
});

test("Live without a public DSN installs no reporter or third-party script", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  const bodies = await reportingPage(page, false);
  await page.evaluate(() => {
    setTimeout(() => {
      throw new Error("unconfigured error");
    }, 0);
  });
  await page.waitForTimeout(50);
  expect(bodies).toEqual([]);
  expect(requests).toEqual(["https://app.test/", "https://app.test/runtime.js"]);
});

test("Live reports a startup failure and removes its partial reporter", async ({ page }) => {
  const bodies = await reportingPage(page, true, true);
  await expect.poll(() => bodies.length).toBe(1);
  expect(bodies[0]).toContain("extension_registration_shape");
  await page.evaluate(() => {
    setTimeout(() => {
      throw new Error("after failed startup");
    }, 0);
  });
  await page.waitForTimeout(50);
  expect(bodies).toHaveLength(1);
});
