import { createHash } from "node:crypto";

import { expect, test, type Page } from "@playwright/test";

import { ISLAND_SELECTOR, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

const FRAMEWORK_ORIGIN = "http://127.0.0.1:4177";
const ASSET_PREFIX = "/__live/v1/assets/";
const RUNTIME_SYMBOL = "suprnova.live.runtime.v1";

interface AssetManifest {
  readonly assets: readonly {
    readonly file: string;
    readonly role: string;
    readonly sri: string;
  }[];
}

async function assetIdentity(page: Page): Promise<string> {
  const response = await page.request.get(`${FRAMEWORK_ORIGIN}/identity`);
  expect(response.status()).toBe(200);
  return (await response.text()).trim();
}

async function manifest(page: Page, identity: string): Promise<AssetManifest> {
  const response = await page.request.get(
    `${FRAMEWORK_ORIGIN}${ASSET_PREFIX}${identity}/suprnova-live.assets.json`,
  );
  expect(response.status()).toBe(200);
  return (await response.json()) as AssetManifest;
}

async function runtimeIsInstalled(page: Page): Promise<boolean> {
  return page.evaluate((symbol) => {
    const runtime: unknown = Reflect.get(window, Symbol.for(symbol));
    return (typeof runtime === "object" || typeof runtime === "function") && runtime !== null;
  }, RUNTIME_SYMBOL);
}

async function expectAllIslandsConnected(page: Page, count: number): Promise<void> {
  const islands = page.locator(ISLAND_SELECTOR);
  await expect(islands).toHaveCount(count);
  for (let index = 0; index < count; index += 1) {
    await expect(islands.nth(index)).toHaveAttribute(STATUS_ATTRIBUTE, "connected");
  }
}

test("SSR content is visible and every island connects through the ESM strategy", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith(ASSET_PREFIX)) requests.push(url.pathname);
  });
  await page.goto(`${FRAMEWORK_ORIGIN}/esm`);
  await expect(page.getByText("Server-rendered bootstrap host", { exact: true })).toBeVisible();
  await expectAllIslandsConnected(page, 3);
  expect(await runtimeIsInstalled(page)).toBe(true);

  const identity = await assetIdentity(page);
  const loaded = new Set(requests.map((path) => path.slice(path.lastIndexOf("/") + 1)));
  expect(loaded).toEqual(
    new Set([
      "suprnova-live.esm.js",
      "suprnova-live.uploads.esm.js",
      "suprnova-live.async.esm.js",
      "suprnova-live.boot.esm.js",
    ]),
  );
  for (const path of requests) {
    expect(path.startsWith(`${ASSET_PREFIX}${identity}/`)).toBe(true);
  }
  const html = await page.content();
  expect(html).not.toContain(".classic.js");
  expect(html).not.toContain("stimulus");
  expect(html.match(/<script(?![^>]*\bsrc=)(?![^>]*type="application\/json")/gu)).toBeNull();
});

test("the classic strategy connects the same islands with deferred scripts", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith(ASSET_PREFIX)) requests.push(url.pathname);
  });
  await page.goto(`${FRAMEWORK_ORIGIN}/classic`);
  await expectAllIslandsConnected(page, 3);
  const loaded = new Set(requests.map((path) => path.slice(path.lastIndexOf("/") + 1)));
  expect(loaded).toEqual(
    new Set([
      "suprnova-live.classic.js",
      "suprnova-live.uploads.classic.js",
      "suprnova-live.async.classic.js",
      "suprnova-live.boot.classic.js",
    ]),
  );
  expect(await page.evaluate(() => typeof Reflect.get(window, "SuprnovaLive"))).toBe("object");
});

test("a core-only document loads no optional role", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith(ASSET_PREFIX)) requests.push(url.pathname);
  });
  await page.goto(`${FRAMEWORK_ORIGIN}/core-only`);
  await expectAllIslandsConnected(page, 2);
  const loaded = new Set(requests.map((path) => path.slice(path.lastIndexOf("/") + 1)));
  expect(loaded).toEqual(new Set(["suprnova-live.esm.js", "suprnova-live.boot.esm.js"]));
});

test("the optional Stimulus bridge loads on request without starting a second runtime", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith(ASSET_PREFIX)) requests.push(url.pathname);
  });
  await page.goto(`${FRAMEWORK_ORIGIN}/stimulus`);
  await expectAllIslandsConnected(page, 2);
  expect(requests.some((path) => path.endsWith("suprnova-live.stimulus.esm.js"))).toBe(true);
  expect(await runtimeIsInstalled(page)).toBe(true);
});

test("duplicate bootstrap tags do not connect islands twice", async ({ page }) => {
  await page.goto(`${FRAMEWORK_ORIGIN}/duplicate`);
  await expectAllIslandsConnected(page, 2);
  const boots = await page.evaluate(
    () => document.querySelectorAll('script[src$="suprnova-live.boot.esm.js"]').length,
  );
  expect(boots).toBe(2);
  const connections = await page.evaluate(() => {
    return Array.from(document.querySelectorAll("[data-suprnova-live-island]")).map((island) =>
      island.getAttribute("data-suprnova-live-status"),
    );
  });
  expect(connections).toEqual(["connected", "connected"]);
  expect(await runtimeIsInstalled(page)).toBe(true);
});

test("an incompatible optional feature fails only its role", async ({ page }) => {
  await page.goto(`${FRAMEWORK_ORIGIN}/incompatible-async`);
  await expectAllIslandsConnected(page, 2);
  const outcomes = await page.evaluate(() => {
    const value: unknown = Reflect.get(window, "__suprnovaFrameworkIncompatible");
    return Array.isArray(value) ? (value as string[]) : [];
  });
  expect(outcomes).toEqual(["async:incompatible"]);
  expect(await runtimeIsInstalled(page)).toBe(true);
});

test("an integrity failure leaves the SSR content intact and the runtime unstarted", async ({
  page,
}) => {
  await page.goto(`${FRAMEWORK_ORIGIN}/integrity-failure`);
  await expect(page.getByText("Server-rendered bootstrap host", { exact: true })).toBeVisible();
  await expect(page.locator(ISLAND_SELECTOR)).toHaveCount(2);
  await page.waitForLoadState("networkidle");
  await expect(page.locator(ISLAND_SELECTOR).first()).not.toHaveAttribute(STATUS_ATTRIBUTE, /.+/u);
  expect(await runtimeIsInstalled(page)).toBe(false);
});

test("a strict self-only Content Security Policy permits startup", async ({ page }) => {
  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy")) violations.push(message.text());
  });
  const response = await page.goto(`${FRAMEWORK_ORIGIN}/csp`);
  expect(response?.headers()["content-security-policy"]).toBe(
    "default-src 'none'; script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'",
  );
  await expectAllIslandsConnected(page, 3);
  expect(violations).toEqual([]);
});

test("artifacts are immutable, validated, and exactly the reviewed bytes", async ({ page }) => {
  const identity = await assetIdentity(page);
  const recorded = await manifest(page, identity);
  for (const asset of recorded.assets) {
    const response = await page.request.get(
      `${FRAMEWORK_ORIGIN}${ASSET_PREFIX}${identity}/${asset.file}`,
    );
    expect(response.status()).toBe(200);
    expect(response.headers()["cache-control"]).toBe("public, max-age=31536000, immutable");
    expect(response.headers()["content-type"]).toBe("text/javascript; charset=utf-8");
    const body = await response.body();
    const digest = createHash("sha256").update(body).digest("base64");
    expect(`sha256-${digest}`).toBe(asset.sri);
    const etag = response.headers()["etag"];
    expect(etag).toBeTruthy();
    const conditional = await page.request.get(
      `${FRAMEWORK_ORIGIN}${ASSET_PREFIX}${identity}/${asset.file}`,
      { headers: { "if-none-match": etag ?? "" } },
    );
    expect(conditional.status()).toBe(304);
  }
  const stale = await page.request.get(
    `${FRAMEWORK_ORIGIN}${ASSET_PREFIX}stale/suprnova-live.esm.js`,
  );
  expect(stale.status()).toBe(404);
});
