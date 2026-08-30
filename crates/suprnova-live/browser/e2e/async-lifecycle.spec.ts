import { expect, test, type Page } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

// Chromium disables the back/forward cache while Playwright tracing is active.
test.use({ trace: "off" });

const ASYNC_SCENARIO = "http://127.0.0.1:4174/scenario/asyncLifecycle";

interface AsyncLifecycleSnapshot {
  readonly activeConnections: number;
  readonly announcements: readonly string[];
  readonly authorizations: number;
  readonly authorizationCompletions: number;
  readonly authorizationFailures: number;
  readonly authorityTrace: readonly unknown[];
  readonly closedConnections: number;
  readonly closeSignals: number;
  readonly connections: number;
  readonly continuityProofs: number;
  readonly cspViolations: readonly Readonly<{ blocked: string; directive: string }>[];
  readonly currentSignals: number;
  readonly degradedSignals: number;
  readonly effectCountsAtDegraded: readonly string[];
  readonly lateMessages: number;
  readonly liveActions: number;
  readonly liveRegionMutations: readonly string[];
  readonly lateCallbackAttempts: Readonly<{
    authorization: number;
    envelope: number;
    membershipAck: number;
  }>;
  readonly pendingLateCallbacks: Readonly<{
    authorizations: number;
    membershipAcks: number;
  }>;
  readonly membershipControls: number;
  readonly pagehidePersisted: readonly boolean[];
  readonly pageshowPersisted: readonly boolean[];
  readonly states: readonly string[];
  readonly runtimeResources: Readonly<{
    authorization: number;
    buffer: number;
    controller: number;
    extension: number;
    listener: number;
    membership: number;
    observer: number;
    queue: number;
    scheduler: number;
    signal: number;
    timer: number;
    transition: number;
    transport: number;
  }>;
  readonly resources: Readonly<{
    activeAuthorizations: number;
    buffers: number;
    connections: number;
    listeners: number;
    observers: number;
    queuedWork: number;
    timers: number;
  }>;
  readonly resourcePeaks: Readonly<{
    activeAuthorizations: number;
    buffers: number;
    connections: number;
    listeners: number;
    observers: number;
    queuedWork: number;
    timers: number;
  }>;
}

function stableLateProjection(snapshot: AsyncLifecycleSnapshot): unknown {
  return {
    authorizationCompletions: snapshot.authorizationCompletions,
    authorizationFailures: snapshot.authorizationFailures,
    authorizations: snapshot.authorizations,
    authorityTrace: snapshot.authorityTrace,
    continuityProofs: snapshot.continuityProofs,
    currentSignals: snapshot.currentSignals,
    liveActions: snapshot.liveActions,
    membershipControls: snapshot.membershipControls,
    states: snapshot.states,
  };
}

async function lifecycleCommand(page: Page, name: string): Promise<void> {
  await page.evaluate(async (command) => {
    const probe: unknown = Reflect.get(window, "__suprnovaAsyncLifecycle");
    if (typeof probe !== "object" || probe === null) throw new Error("async_probe_missing");
    const callback: unknown = Reflect.get(probe, command);
    if (typeof callback !== "function") throw new Error(`async_probe_${command}_missing`);
    await Reflect.apply(callback, probe, []);
  }, name);
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
  await page.goto(ASYNC_SCENARIO);

  const island = page.locator("[data-suprnova-live-island]");
  await expectCurrentSignal(page, 1);
  await expect(island).toHaveAttribute("data-live-stream-motion", "reduced");
  await expect(page.getByRole("status", { name: "Order updates" })).toContainText(
    "Updates current",
  );
  const baselineRuntimeResources = (await lifecycleSnapshot(page)).runtimeResources;

  await page.getByRole("button", { name: "Keep focus" }).focus();
  await page.getByRole("button", { name: "Degrade stream" }).click();
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).states.includes("degraded"))
    .toBe(true);
  await expect(page.getByRole("button", { name: "Degrade stream" })).toBeFocused();
  expect((await lifecycleSnapshot(page)).effectCountsAtDegraded).toContain("0");
  for (let cycle = 0; cycle < 3; cycle += 1) {
    const beforeReconnect = await lifecycleSnapshot(page);
    await page.getByRole("button", { name: "Reconnect stream" }).click();
    await expect
      .poll(async () => (await lifecycleSnapshot(page)).connections)
      .toBeGreaterThan(beforeReconnect.connections);
    await expect
      .poll(async () => (await lifecycleSnapshot(page)).membershipControls)
      .toBeGreaterThan(beforeReconnect.membershipControls);
    await expect.poll(async () => (await lifecycleSnapshot(page)).activeConnections).toBe(1);
    await expectCurrentSignal(page, cycle + 2);
    await expect
      .poll(async () => (await lifecycleSnapshot(page)).runtimeResources)
      .toEqual(baselineRuntimeResources);
    if (cycle < 2) {
      const degradedBefore = (await lifecycleSnapshot(page)).degradedSignals;
      const effectBeforeDegrade = await page.locator("#async-effect-count").textContent();
      await page.getByRole("button", { name: "Degrade stream" }).click();
      await expect
        .poll(async () => (await lifecycleSnapshot(page)).degradedSignals)
        .toBeGreaterThan(degradedBefore);
      const effectEvidence = (await lifecycleSnapshot(page)).effectCountsAtDegraded;
      expect(effectEvidence[effectEvidence.length - 1]).toBe(effectBeforeDegrade);
    }
  }

  const reconnectSnapshot = await lifecycleSnapshot(page);
  expect(reconnectSnapshot.states).toEqual(
    expect.arrayContaining(["disconnected", "connecting", "current", "degraded", "reconnecting"]),
  );
  expect(reconnectSnapshot.announcements.length).toBeLessThanOrEqual(8);
  expect(reconnectSnapshot.liveRegionMutations.length).toBeGreaterThan(
    reconnectSnapshot.announcements.length,
  );
  expect(reconnectSnapshot.cspViolations).toEqual([]);
  expect(reconnectSnapshot.resourcePeaks).toMatchObject({
    activeAuthorizations: 1,
    buffers: 1,
    connections: 1,
    listeners: 4,
    observers: 2,
  });
  expect(reconnectSnapshot.resourcePeaks.queuedWork).toBeGreaterThanOrEqual(1);
  expect(reconnectSnapshot.resourcePeaks.timers).toBeGreaterThanOrEqual(1);
  expect(reconnectSnapshot.runtimeResources.listener).toBeGreaterThan(0);
  expect(reconnectSnapshot.runtimeResources.observer).toBeGreaterThan(0);
  expect(reconnectSnapshot.runtimeResources).toMatchObject({
    authorization: 0,
    buffer: 1,
    membership: 0,
    queue: 0,
    transport: 1,
  });
  expect(reconnectSnapshot.runtimeResources.timer).toBeGreaterThanOrEqual(1);
  expect(reconnectSnapshot.activeConnections).toBe(1);
  expect(reconnectSnapshot.closedConnections).toBe(reconnectSnapshot.connections - 1);
  await expectNoSeriousA11yViolations(page, {
    sourceUrl: "http://127.0.0.1:4173/test-vendor/axe.js",
  });
  expect((await lifecycleSnapshot(page)).cspViolations).toEqual([]);
});

test("server completion exposes a closed status and retires the physical stream", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto(ASYNC_SCENARIO);
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
  await page.goto(ASYNC_SCENARIO);
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

  const beforeLate = await lifecycleSnapshot(page);
  const contentBeforeLate = await page.locator("#async-content").textContent();
  const effectBeforeLate = await page.locator("#async-effect-count").textContent();
  await lifecycleCommand(page, "injectLate");
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).lateCallbackAttempts.envelope)
    .toBeGreaterThanOrEqual(1);
  const restored = await lifecycleSnapshot(page);
  expect(stableLateProjection(restored)).toEqual(stableLateProjection(beforeLate));
  await expect(page.locator("#async-content")).toHaveText(contentBeforeLate ?? "");
  await expect(page.locator("#async-effect-count")).toHaveText(effectBeforeLate ?? "");
  expect(restored.pagehidePersisted).toContain(true);
  expect(restored.pageshowPersisted).toContain(true);
  expect(restored.closedConnections).toBe(before.closedConnections + 1);
  expect(restored.connections).toBe(before.connections + 1);
  expect(restored.authorizations).toBe(before.authorizations + 1);
  expect(restored.continuityProofs).toBeGreaterThan(before.continuityProofs);
  expect(restored.activeConnections).toBe(1);
  expect(restored.lateMessages).toBe(
    restored.lateCallbackAttempts.authorization +
      restored.lateCallbackAttempts.envelope +
      restored.lateCallbackAttempts.membershipAck,
  );
  expect(restored.resources).toMatchObject({
    activeAuthorizations: 0,
    buffers: 0,
    connections: 1,
    listeners: 4,
    observers: 2,
    queuedWork: 0,
  });
  expect(restored.cspViolations).toEqual([]);
  expect(restored.lateCallbackAttempts.envelope).toBeGreaterThanOrEqual(1);
  expect(restored.runtimeResources).toEqual(before.runtimeResources);
});

test("native controls and local signals remain available beside async updates", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto(ASYNC_SCENARIO);
  await expectCurrentSignal(page, 1);

  await page.getByRole("button", { name: "Local details" }).click();
  await expect(page.getByText("Local signal remains available")).toBeVisible();
  await lifecycleCommand(page, "armLateAuthorization");
  await lifecycleCommand(page, "retirePush");
  await expect.poll(async () => (await lifecycleSnapshot(page)).activeConnections).toBe(0);
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).pendingLateCallbacks.authorizations)
    .toBe(1);
  await expect(page.locator("[data-suprnova-live-island]")).not.toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await page.getByRole("button", { name: "Run Live action" }).click();
  await expect(page.locator("#async-action-result")).toHaveText("Live action committed");
  await expect.poll(async () => (await lifecycleSnapshot(page)).liveActions).toBe(1);
  await page.getByRole("textbox", { name: "Native value" }).fill("ordinary-http");
  await page.getByRole("button", { name: "Submit normally" }).click();
  await expect(page).toHaveURL(/\/navigation\/post$/u);
  await expect(page.locator("#post-body")).toContainText("value=ordinary-http");
});

test("morph replacement and island removal retire async resources exactly once", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto(ASYNC_SCENARIO);
  const island = page.locator("[data-suprnova-live-island]");
  await expectCurrentSignal(page, 1);

  await lifecycleCommand(page, "armLateAuthorization");
  await lifecycleCommand(page, "retirePush");
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).pendingLateCallbacks.authorizations)
    .toBe(1);

  await page.getByRole("button", { name: "Replace island contents" }).click();
  await expect(page.getByText("Morphed async content")).toBeVisible();
  await expect(island).not.toHaveAttribute("live:stream", /.+/u);
  await expect(island).toHaveAttribute("live:poll", "");
  await expect(island).not.toHaveAttribute("data-live-stream-state", /.+/u);
  await expect(island).not.toHaveAttribute("data-live-stream-motion", /.+/u);
  await expect(island).toHaveAttribute("aria-busy", "false");
  await expect(page.getByText("Server rendered status baseline")).toBeVisible();
  await page.getByRole("button", { name: "Remove island" }).click();
  await expect(island).toHaveCount(0);
  const beforeLate = await lifecycleSnapshot(page);
  await lifecycleCommand(page, "injectLate");
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).lateMessages)
    .toBeGreaterThanOrEqual(2);

  const retired = await lifecycleSnapshot(page);
  expect(stableLateProjection(retired)).toEqual(stableLateProjection(beforeLate));
  await expect(island).toHaveCount(0);
  expect(retired.activeConnections).toBe(0);
  expect(retired.closedConnections).toBe(retired.connections);
  expect(retired.lateMessages).toBe(
    retired.lateCallbackAttempts.authorization +
      retired.lateCallbackAttempts.envelope +
      retired.lateCallbackAttempts.membershipAck,
  );
  expect(retired.lateCallbackAttempts.authorization).toBe(1);
  expect(retired.lateCallbackAttempts.envelope).toBeGreaterThanOrEqual(1);
  expect(retired.resources).toMatchObject({
    activeAuthorizations: 0,
    buffers: 0,
    connections: 0,
    listeners: 4,
    observers: 2,
    queuedWork: 0,
    timers: 0,
  });
  expect(retired.runtimeResources).toMatchObject({
    authorization: 0,
    buffer: 0,
    membership: 0,
    queue: 0,
    timer: 0,
    transport: 0,
  });
});

test("document shutdown is idempotent and leaves no async resource authority", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Covered by the normal Chromium project.");
  await page.goto(ASYNC_SCENARIO);
  await expectCurrentSignal(page, 1);

  await lifecycleCommand(page, "armLateMembershipAck");
  await lifecycleCommand(page, "retirePush");
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).pendingLateCallbacks.membershipAcks)
    .toBe(1);

  await page.evaluate(async () => {
    const probe: unknown = Reflect.get(window, "__suprnovaAsyncLifecycle");
    if (typeof probe !== "object" || probe === null) throw new Error("async_probe_missing");
    const shutdown: unknown = Reflect.get(probe, "shutdown");
    if (typeof shutdown !== "function") throw new Error("async_probe_shutdown_missing");
    await Reflect.apply(shutdown, probe, []);
    await Reflect.apply(shutdown, probe, []);
  });

  const beforeLate = await lifecycleSnapshot(page);
  await lifecycleCommand(page, "injectLate");
  await expect
    .poll(async () => (await lifecycleSnapshot(page)).lateCallbackAttempts.membershipAck)
    .toBe(1);
  const stopped = await lifecycleSnapshot(page);
  expect(stableLateProjection(stopped)).toEqual(stableLateProjection(beforeLate));
  expect(stopped.activeConnections).toBe(0);
  expect(stopped.closedConnections).toBe(stopped.connections);
  expect(stopped.lateMessages).toBe(
    stopped.lateCallbackAttempts.authorization +
      stopped.lateCallbackAttempts.envelope +
      stopped.lateCallbackAttempts.membershipAck,
  );
  expect(stopped.lateCallbackAttempts.membershipAck).toBe(1);
  expect(stopped.lateCallbackAttempts.envelope).toBeGreaterThanOrEqual(1);
  expect(stopped.resources).toMatchObject({
    activeAuthorizations: 0,
    buffers: 0,
    connections: 0,
    listeners: 4,
    observers: 2,
    queuedWork: 0,
    timers: 0,
  });
  expect(stopped.runtimeResources).toEqual({
    authorization: 0,
    buffer: 0,
    controller: 0,
    extension: 0,
    listener: 0,
    membership: 0,
    observer: 0,
    queue: 0,
    scheduler: 0,
    signal: 0,
    timer: 0,
    transition: 0,
    transport: 0,
  });
  const island = page.locator("[data-suprnova-live-island]");
  await expect(island).toHaveAttribute("aria-busy", "false");
  await expect(island).toHaveAttribute("data-live-stream-state", "disconnected");
  await expect(island).toHaveAttribute("data-live-stream-motion", "allowed");
});
