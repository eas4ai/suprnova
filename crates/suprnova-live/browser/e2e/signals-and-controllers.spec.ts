import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("signal and Stimulus identity preserve, insert, remove, and reset exactly once", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("continuity");
  await page.locator("#continuity-toggle").click();
  await expect(page.locator("#continuity-signal-state")).toBeVisible();
  await page.evaluate(() => {
    Reflect.set(window, "__continuitySignal", document.querySelector("#continuity-signal"));
    const action = document.querySelector("#continuity-action");
    if (!(action instanceof HTMLElement)) throw new Error("action missing");
    action.click();
  });

  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#continuity-signal-state")).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        Reflect.get(window, "__continuitySignal") === document.querySelector("#continuity-signal"),
    ),
  ).toBe(true);
  expect(
    JSON.parse((await page.locator("html").getAttribute("data-continuity-lifecycle")) ?? "{}"),
  ).toEqual({
    connect: { inserted: 1, preserved: 1, removed: 1 },
    disconnect: { removed: 1 },
  });

  await page.locator("#continuity-action").click();
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "9");
  await expect(page.locator("#continuity-signal-state")).toBeHidden();
  expect(
    await page.evaluate(
      () =>
        Reflect.get(window, "__continuitySignal") === document.querySelector("#continuity-signal"),
    ),
  ).toBe(false);
  expect(
    JSON.parse((await page.locator("html").getAttribute("data-continuity-lifecycle")) ?? "{}"),
  ).toEqual({
    connect: { inserted: 1, preserved: 1, removed: 1 },
    disconnect: { removed: 1 },
  });

  await page.evaluate(() => {
    Reflect.set(window, "__continuityResetSignal", document.querySelector("#continuity-signal"));
  });
  await page.locator("#continuity-action").click();
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "10");
  await expect(page.locator("#continuity-signal")).toHaveCount(0);
  expect(
    await page.evaluate(() => {
      const scope = (window as typeof window & { __continuityResetSignal?: unknown })
        .__continuityResetSignal;
      return scope instanceof Element ? scope.isConnected : null;
    }),
  ).toBe(false);
  expect(
    JSON.parse((await page.locator("html").getAttribute("data-continuity-lifecycle")) ?? "{}"),
  ).toEqual({
    connect: { inserted: 1, preserved: 1, removed: 1 },
    disconnect: { removed: 1 },
  });
});
