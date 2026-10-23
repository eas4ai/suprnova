import { expect, test } from "@playwright/test";

async function lifecycleStatus(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaLifecycleProbe");
    if (typeof probe !== "object" || probe === null) throw new Error("lifecycle_probe_missing");
    const runtime: unknown = Reflect.get(probe, "runtime");
    if (typeof runtime !== "object" || runtime === null)
      throw new Error("lifecycle_runtime_missing");
    const status: unknown = Reflect.get(runtime, "status");
    if (typeof status !== "function") throw new Error("lifecycle_status_missing");
    return String(Reflect.apply(status, runtime, []));
  });
}

async function dispatchTransition(
  page: import("@playwright/test").Page,
  type: "pagehide" | "pageshow",
  persisted: boolean,
): Promise<void> {
  await page.evaluate(
    ({ persisted, type }) => {
      const event = new Event(type);
      Object.defineProperty(event, "persisted", { value: persisted });
      window.dispatchEvent(event);
    },
    { persisted, type },
  );
}

test("persisted hide, freeze, pageshow, and resume restore once without duplicate boot", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );

  await dispatchTransition(page, "pagehide", true);
  await page.evaluate(() => document.dispatchEvent(new Event("freeze")));
  expect(await lifecycleStatus(page)).toBe("suspended");
  await dispatchTransition(page, "pageshow", true);
  await dispatchTransition(page, "pageshow", true);
  await page.evaluate(() => document.dispatchEvent(new Event("resume")));

  expect(await lifecycleStatus(page)).toBe("running");
  expect(
    await page.evaluate(() => {
      const probe: unknown = Reflect.get(window, "__suprnovaLifecycleProbe");
      if (typeof probe !== "object" || probe === null) return false;
      const bootAgain: unknown = Reflect.get(probe, "bootAgain");
      if (typeof bootAgain !== "function") return false;
      Reflect.apply(bootAgain, probe, []);
      return true;
    }),
  ).toBe(true);
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
});

test("restore rejects changed asset identity while retaining visible SSR HTML", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await dispatchTransition(page, "pagehide", true);
  await page.evaluate(() => {
    const config = document.querySelector("#suprnova-live-config");
    if (config === null) throw new Error("config_missing");
    const value: unknown = JSON.parse(config.textContent);
    if (typeof value !== "object" || value === null) throw new Error("config_invalid");
    Reflect.set(value, "asset_identity", "incompatible-restored-asset");
    config.textContent = JSON.stringify(value);
  });
  await dispatchTransition(page, "pageshow", true);

  expect(await lifecycleStatus(page)).toBe("stopped");
  await expect(page.locator("#lifecycle-content")).toHaveText("Lifecycle original");
  await expect(page.locator("#lifecycle-content")).toBeVisible();
});

test("restore rejects changed runtime contract while retaining visible SSR HTML", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await dispatchTransition(page, "pagehide", true);
  await page.evaluate(() => {
    const config = document.querySelector("#suprnova-live-config");
    if (config === null) throw new Error("config_missing");
    const value: unknown = JSON.parse(config.textContent);
    if (typeof value !== "object" || value === null) throw new Error("config_invalid");
    Reflect.set(value, "runtime_contract_version", 2);
    config.textContent = JSON.stringify(value);
  });
  await dispatchTransition(page, "pageshow", true);

  expect(await lifecycleStatus(page)).toBe("stopped");
  await expect(page.locator("#lifecycle-content")).toHaveText("Lifecycle original");
});

test("restore rejects changed protocol range while retaining visible SSR HTML", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await dispatchTransition(page, "pagehide", true);
  await page.evaluate(() => {
    const config = document.querySelector("#suprnova-live-config");
    if (config === null) throw new Error("config_missing");
    const value: unknown = JSON.parse(config.textContent);
    if (typeof value !== "object" || value === null) throw new Error("config_invalid");
    Reflect.set(value, "protocol", { maximum: 1, minimum: 1 });
    config.textContent = JSON.stringify(value);
  });
  await dispatchTransition(page, "pageshow", true);

  expect(await lifecycleStatus(page)).toBe("stopped");
  await expect(page.locator("#lifecycle-content")).toHaveText("Lifecycle original");
});

test("restore rejects incompatible island metadata while retaining visible SSR HTML", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await dispatchTransition(page, "pagehide", true);
  await page.locator("[data-suprnova-live-island]").evaluate((island) => {
    island.setAttribute("data-suprnova-live-protocol-min", "3");
  });
  await dispatchTransition(page, "pageshow", true);

  expect(await lifecycleStatus(page)).toBe("stopped");
  await expect(page.locator("#lifecycle-content")).toHaveText("Lifecycle original");
});

test("native Back and Forward keep one usable runtime whether restored or replaced", async ({
  page,
}) => {
  await page.goto("/scenario/lifecycle");
  await expect(page.locator("#lifecycle-content")).toBeVisible();
  const token = await page.locator("html").getAttribute("data-lifecycle-token");
  await page.goto("/scenario/lifecycleDestination");
  await page.goBack();
  await expect(page.locator("#lifecycle-content")).toBeVisible();
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
  const restoredToken = await page.locator("html").getAttribute("data-lifecycle-token");
  expect(restoredToken).not.toBeNull();
  if (restoredToken === token) expect(await lifecycleStatus(page)).toBe("running");
  await page.goForward();
  await expect(page.getByRole("heading", { name: "Lifecycle destination" })).toBeVisible();
});

test("non-persisted pagehide disposes behavior without an unload dependency", async ({ page }) => {
  await page.goto("/scenario/lifecycle");
  await dispatchTransition(page, "pagehide", false);
  expect(await lifecycleStatus(page)).toBe("stopped");
  await expect(page.locator("#lifecycle-content")).toBeVisible();
});
