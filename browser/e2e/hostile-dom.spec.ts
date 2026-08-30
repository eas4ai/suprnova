import { expect, test } from "@playwright/test";

import { browserTaskBarrier } from "./support/event-loop-barrier.js";

for (const scenario of [
  "hostileMalformedUtf8",
  "hostileHugeJson",
  "hostilePrototypeKey",
] as const) {
  test(`${scenario} retains accepted DOM and bounded authority`, async ({ page }) => {
    await page.goto(`/scenario/${scenario}`);
    const island = page.locator('[data-suprnova-live-document-key="primary"]');
    const response = page.waitForResponse(
      (candidate) => new URL(candidate.url()).pathname === "/live",
    );
    await page.locator("#hostile-action").click();
    await response;
    await browserTaskBarrier(page);

    await expect(island).toHaveAttribute("data-suprnova-live-revision", "7");
    await expect(page.locator("#hostile-original")).toHaveText("Last accepted hostile fixture");
    await expect(island).toHaveAttribute("data-suprnova-live-status", "connected");
  });
}

for (const scenario of ["hostileExtremeMorph", "hostileDuplicateIdentity"] as const) {
  test(`${scenario} fails preflight without partial mutation or replay`, async ({ page }) => {
    await page.goto(`/scenario/${scenario}`);
    const island = page.locator('[data-suprnova-live-document-key="primary"]');
    await page.locator("#hostile-action").click();

    await expect(island).toHaveAttribute("data-suprnova-live-status", "disconnected");
    await expect(island).toHaveAttribute("data-suprnova-live-revision", "7");
    await expect(page.locator("#hostile-original")).toHaveText("Last accepted hostile fixture");
  });
}

test("extreme initial depth, count, attributes, and text stop at one visible closed outcome", async ({
  page,
}) => {
  let requests = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") requests += 1;
  });
  await page.goto("/scenario/hostileInitialLimits");
  const island = page.locator('[data-suprnova-live-document-key="primary"]');
  await expect(page.locator("#hostile-limit-marker")).toBeVisible();
  await page.locator("#hostile-over-limit").click();
  await browserTaskBarrier(page);
  await expect(island).toHaveAttribute("data-suprnova-live-status", "connected");
  expect(requests).toBe(0);
});

test("returned scripts and event handlers never execute or partially mutate accepted DOM", async ({
  page,
}) => {
  await page.goto("/scenario/morphUnsafe");
  await page.locator("#morph-unsafe-action").click();
  await expect(page.locator("html")).not.toHaveAttribute("data-morph-script-executed", "true");
  await expect(page.locator("html")).not.toHaveAttribute("data-morph-handler-executed", "true");
  await expect(page.locator("#morph-unsafe-content")).toHaveText("Original");
});

test("third-party mutation, shadow ownership, and duplicate roots stay bounded", async ({
  page,
}) => {
  let requests = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") requests += 1;
  });
  await page.goto("/scenario/directiveOwnership");
  await page.locator("#child-plain").evaluate((element) => {
    element.setAttribute("live:click.prevent", "forged");
  });
  await page.locator("#child-plain").click();
  await page.evaluate(() => {
    const closedButton: unknown = Reflect.get(window, "__suprnovaClosedButton");
    if (!(closedButton instanceof HTMLButtonElement))
      throw new Error("closed_shadow_fixture_missing");
    closedButton.click();
  });
  await browserTaskBarrier(page);
  expect(requests).toBe(0);

  const openRequest = page.waitForRequest((request) => new URL(request.url()).pathname === "/live");
  await page.locator("#open-host").evaluate((element) => {
    const openButton = element.shadowRoot?.querySelector("button");
    if (!(openButton instanceof HTMLButtonElement)) throw new Error("open_shadow_fixture_missing");
    openButton.click();
    element.remove();
  });
  await openRequest;
  expect(requests).toBe(1);

  await page.goto("/scenario/duplicate");
  await expect(page.locator('[data-suprnova-live-status="connected"]')).toHaveCount(1);
  await expect(page.locator('[data-suprnova-live-status="invalid"]')).toHaveCount(1);
});

test("throwing getters and proxies at public APIs reject without stopping the runtime", async ({
  page,
}) => {
  await page.goto("/scenario/effects");
  const result = await page.evaluate(async () => {
    const runtime: unknown = Reflect.get(window, "__suprnovaExtensionRuntime");
    const call = document.querySelector("#extension-call");
    const root = document.querySelector("[data-suprnova-live-island]");
    if (typeof runtime !== "object" || runtime === null || call === null || root === null) {
      throw new Error("hostile_api_fixture_missing");
    }
    const invokeCall: unknown = Reflect.get(runtime, "call");
    const runEffect: unknown = Reflect.get(runtime, "runEffect");
    const status: unknown = Reflect.get(runtime, "status");
    if (
      typeof invokeCall !== "function" ||
      typeof runEffect !== "function" ||
      typeof status !== "function"
    ) {
      throw new Error("hostile_api_fixture_invalid");
    }
    const hostile = new Proxy(
      {},
      {
        get() {
          throw new Error("hostile_getter");
        },
      },
    );
    let callRejected = false;
    let effectRejected = false;
    try {
      await Reflect.apply(invokeCall, runtime, [call, "mark-ready", hostile]);
    } catch {
      callRejected = true;
    }
    try {
      await Reflect.apply(runEffect, runtime, [root, hostile]);
    } catch {
      effectRejected = true;
    }
    return {
      callRejected,
      effectRejected,
      status: String(Reflect.apply(status, runtime, [])),
    };
  });

  expect(result).toEqual({ callRejected: true, effectRejected: true, status: "running" });
});
