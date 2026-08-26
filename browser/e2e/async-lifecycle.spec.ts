import { expect, test, type Page } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

// Chromium disables the back/forward cache while Playwright tracing is active.
test.use({ trace: "off" });

interface AsyncLifecycleSnapshot {
  readonly activeConnections: number;
  readonly announcements: readonly string[];
  readonly authorizations: number;
  readonly authorizationCompletions: number;
  readonly authorizationFailures: number;
  readonly closedConnections: number;
  readonly closeSignals: number;
  readonly connections: number;
  readonly continuityProofs: number;
  readonly currentSignals: number;
  readonly lateMessages: number;
  readonly pagehidePersisted: readonly boolean[];
  readonly pageshowPersisted: readonly boolean[];
  readonly states: readonly string[];
}

async function lifecycleSnapshot(page: Page): Promise<AsyncLifecycleSnapshot> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaAsyncLifecycle");
    if (typeof probe !== "object" || probe === null) throw new Error("async_probe_missing");
    const snapshot: unknown = Reflect.get(probe, "snapshot");
    if (typeof snapshot !== "function") throw new Error("async_probe_snapshot_missing");
    return Reflect.apply(snapshot, probe, []) as AsyncLifecycleSnapshot;
  });
}

async function expectCurrentSignal(page: Page, minimum: number): Promise<void> {
  await expect
    .poll(async () => {
      try {
        return (await lifecycleSnapshot(page)).currentSignals;
      } catch {
        return 0;
      }
    })
    .toBeGreaterThanOrEqual(minimum);
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
}

test("real async transport exposes bounded semantic feedback without stealing focus", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/scenario/asyncLifecycle");

  const island = page.locator("[data-suprnova-live-island]");
  await expectCurrentSignal(page, 1);
  await expect(island).toHaveAttribute("data-live-stream-motion", "reduced");
  await expect(page.getByRole("status", { name: "Order updates" })).toContainText(
    "Updates current",
  );

  await page.getByRole("button", { name: "Keep focus" }).focus();
  await page.getByRole("button", { name: "Degrade stream" }).click();
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).states.includes("degraded"))
    .toBe(true);
  await expect(page.getByRole("button", { name: "Degrade stream" })).toBeFocused();
  await page.getByRole("button", { name: "Reconnect stream" }).click();
  await expectCurrentSignal(page, 2);

  const reconnectSnapshot = await lifecycleSnapshot(page);
  expect(reconnectSnapshot.states).toEqual(
    expect.arrayContaining(["disconnected", "connecting", "current", "degraded", "reconnecting"]),
  );
  expect(reconnectSnapshot.announcements.length).toBeLessThanOrEqual(6);
  expect(new Set(reconnectSnapshot.announcements).size).toBe(
    reconnectSnapshot.announcements.length,
  );
  await expectNoSeriousA11yViolations(page, { sourceUrl: "/test-vendor/axe.js" });
});

test("server completion exposes a closed status and retires the physical stream", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto("/scenario/asyncLifecycle");
  await expectCurrentSignal(page, 1);

  const island = page.locator("[data-suprnova-live-island]");
  await page.getByRole("button", { name: "Close stream" }).click();
  await expect.poll(async () => (await lifecycleSnapshot(page)).closeSignals).toBe(1);
  await expect(island).toHaveAttribute("data-live-stream-state", "closed");
  await expect(page.getByRole("status", { name: "Order updates" })).toContainText("Updates closed");
  await expect.poll(async () => (await lifecycleSnapshot(page)).activeConnections).toBe(0);

  const snapshot = await lifecycleSnapshot(page);
  expect(snapshot.states).toContain("closed");
  expect(snapshot.closedConnections).toBe(snapshot.connections);
  expect(snapshot.announcements.length).toBeLessThanOrEqual(6);
  expect(new Set(snapshot.announcements).size).toBe(snapshot.announcements.length);
});

test("actual bfcache restore reauthorizes and proves continuity before accepting data", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "chrome-bfcache",
    "The exact persisted BFCache proof runs in dedicated stable Chrome.",
  );
  await page.goto("/scenario/asyncLifecycle");
  await expectCurrentSignal(page, 1);
  const before = await lifecycleSnapshot(page);

  await page.getByRole("link", { name: "Native destination" }).click();
  await expect(page.getByRole("heading", { name: "Lifecycle destination" })).toBeVisible();
  await page.evaluate(() => {
    history.back();
  });
  await expect
    .poll(async () => {
      try {
        return (await lifecycleSnapshot(page)).pageshowPersisted.includes(true);
      } catch {
        return false;
      }
    })
    .toBe(true);
  await expectCurrentSignal(page, 2);

  const restored = await lifecycleSnapshot(page);
  expect(restored.pagehidePersisted).toContain(true);
  expect(restored.pageshowPersisted).toContain(true);
  expect(restored.closedConnections).toBe(before.closedConnections + 1);
  expect(restored.connections).toBe(before.connections + 1);
  expect(restored.authorizations).toBe(before.authorizations + 1);
  expect(restored.continuityProofs).toBeGreaterThan(before.continuityProofs);
  expect(restored.activeConnections).toBe(1);
  expect(restored.lateMessages).toBe(0);
});

test("native controls and local signals remain available beside async updates", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto("/scenario/asyncLifecycle");
  await expectCurrentSignal(page, 1);

  await page.getByRole("button", { name: "Local details" }).click();
  await expect(page.getByText("Local signal remains available")).toBeVisible();
  await page.getByRole("textbox", { name: "Native value" }).fill("ordinary-http");
  await page.getByRole("button", { name: "Submit normally" }).click();
  await expect(page).toHaveURL(/\/navigation\/post$/u);
  await expect(page.locator("#post-body")).toContainText("value=ordinary-http");
});

test("morph replacement and island removal retire async resources exactly once", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto("/scenario/asyncLifecycle");
  const island = page.locator("[data-suprnova-live-island]");
  await expectCurrentSignal(page, 1);

  await page.getByRole("button", { name: "Replace island contents" }).click();
  await expect(page.getByText("Morphed async content")).toBeVisible();
  await expect(island).toHaveAttribute("data-live-stream-state", "current");
  await page.getByRole("button", { name: "Remove island" }).click();
  await expect(island).toHaveCount(0);

  const retired = await lifecycleSnapshot(page);
  expect(retired.activeConnections).toBe(0);
  expect(retired.closedConnections).toBe(retired.connections);
  expect(retired.lateMessages).toBe(0);
});

test("document shutdown is idempotent and leaves no async resource authority", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto("/scenario/asyncLifecycle");
  await expectCurrentSignal(page, 1);

  await page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaAsyncLifecycle");
    if (typeof probe !== "object" || probe === null) throw new Error("async_probe_missing");
    const shutdown: unknown = Reflect.get(probe, "shutdown");
    if (typeof shutdown !== "function") throw new Error("async_probe_shutdown_missing");
    Reflect.apply(shutdown, probe, []);
    Reflect.apply(shutdown, probe, []);
  });

  const stopped = await lifecycleSnapshot(page);
  expect(stopped.activeConnections).toBe(0);
  expect(stopped.closedConnections).toBe(stopped.connections);
  expect(stopped.lateMessages).toBe(0);
  const island = page.locator("[data-suprnova-live-island]");
  await expect(island).toHaveAttribute("aria-busy", "false");
  await expect(island).toHaveAttribute("data-live-stream-state", "disconnected");
  await expect(island).toHaveAttribute("data-live-stream-motion", "allowed");
});
