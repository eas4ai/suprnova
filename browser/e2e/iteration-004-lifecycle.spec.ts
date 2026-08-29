import { expect, test, type Page } from "@playwright/test";

test.use({ trace: "off" });

const SCENARIO =
  "http://127.0.0.1:4175/scenario/iteration004?features=async&format=esm&transport=sse&lifecycle=true&hybrid=true";

interface LifecycleSnapshot {
  readonly authorizations: readonly Readonly<{
    readonly baseline: Readonly<{ readonly epoch: string; readonly sequence: string }>;
    readonly position: Readonly<{ readonly epoch: string; readonly sequence: string }> | null;
    readonly replay: number;
    readonly subscription: string;
    readonly transport: string;
  }>[];
  readonly controlledTimerDelays: readonly number[];
  readonly forwardedEnvelopes: number;
  readonly freshnessStates: readonly string[];
  readonly heldEnvelopes: number;
  readonly host: Readonly<{
    readonly active_physical_transports: number;
    readonly logical_memberships: number;
    readonly physical_sse_connections: number;
  }>;
  readonly pagehidePersisted: readonly boolean[];
  readonly pageshowPersisted: readonly boolean[];
  readonly portsCreated: number;
  readonly retiredEnvelopeAttempts: number;
  readonly subscriptionAttempts: number;
  readonly transportFailures: readonly string[];
  readonly runtimeResources: Readonly<Record<string, number>>;
}

async function snapshot(page: Page): Promise<LifecycleSnapshot> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    if ((typeof probe !== "object" && typeof probe !== "function") || probe === null) {
      throw new Error("iteration_004_probe_missing");
    }
    const inspect: unknown = Reflect.get(probe, "snapshot");
    if (typeof inspect !== "function") throw new Error("iteration_004_snapshot_missing");
    return Reflect.apply(inspect, probe, []) as LifecycleSnapshot;
  });
}

function last<T>(values: readonly T[]): T | undefined {
  return values[values.length - 1];
}

async function command(page: Page, name: string): Promise<void> {
  await page.evaluate(async (commandName) => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    const callback: unknown =
      (typeof probe === "object" || typeof probe === "function") && probe !== null
        ? Reflect.get(probe, commandName)
        : null;
    if (typeof callback !== "function") throw new Error(`iteration_004_${commandName}_missing`);
    const invoke = callback as (this: unknown) => unknown;
    await invoke.call(probe);
  }, name);
}

async function freshnessStates(page: Page): Promise<readonly string[]> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    const callback: unknown =
      (typeof probe === "object" || typeof probe === "function") && probe !== null
        ? Reflect.get(probe, "freshnessStates")
        : null;
    if (typeof callback !== "function") throw new Error("iteration_004_freshness_missing");
    const invoke = callback as (this: unknown) => unknown;
    const result: unknown = invoke.call(probe);
    if (!Array.isArray(result) || !result.every((value: unknown) => typeof value === "string")) {
      throw new Error("iteration_004_freshness_invalid");
    }
    return result;
  });
}

async function waitForHostQuiescent(page: Page, origin = "http://127.0.0.1:4175"): Promise<void> {
  await expect
    .poll(async () => {
      const response = await page.request.get(`${origin}/__test/iteration-004/inspection`);
      if (!response.ok()) return null;
      const value = (await response.json()) as {
        active_physical_transports: number;
        logical_memberships: number;
      };
      return {
        physical: value.active_physical_transports,
        memberships: value.logical_memberships,
      };
    })
    .toEqual({ physical: 0, memberships: 0 });
}

test("real bfcache restoration retires the old physical generation and reauthorizes from current position", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name !== "chrome-bfcache",
    "Persisted PageTransitionEvent proof runs in dedicated stable Chrome.",
  );
  await waitForHostQuiescent(page);
  await page.goto(SCENARIO);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  const beforeHold = await snapshot(page);
  await command(page, "holdNextEnvelope");
  await expect.poll(async () => (await snapshot(page)).heldEnvelopes).toBe(1);
  await expect
    .poll(async () => (await snapshot(page)).forwardedEnvelopes)
    .toBeGreaterThanOrEqual(beforeHold.forwardedEnvelopes + 2);
  const before = await snapshot(page);
  expect(before.host).toMatchObject({
    active_physical_transports: 1,
    logical_memberships: 1,
  });

  await page.getByRole("link", { name: "Ordinary destination" }).click();
  await expect(page.getByRole("heading", { name: "Iteration 004 destination" })).toBeVisible();
  await expect
    .poll(async () => {
      const response = await page.request.get(
        "http://127.0.0.1:4175/__test/iteration-004/inspection",
      );
      return response.ok()
        ? ((await response.json()) as { active_physical_transports: number })
            .active_physical_transports
        : -1;
    })
    .toBe(0);
  await page.goBack({ waitUntil: "commit" });
  await expect
    .poll(async () => {
      try {
        return (await snapshot(page)).pageshowPersisted.includes(true);
      } catch {
        return false;
      }
    })
    .toBe(true);
  await expect
    .poll(async () => (await snapshot(page)).authorizations.length)
    .toBe(before.authorizations.length + 1);
  await expect.poll(async () => (await snapshot(page)).portsCreated).toBe(before.portsCreated + 1);
  await expect
    .poll(async () => (await snapshot(page)).subscriptionAttempts)
    .toBe(before.subscriptionAttempts + 1);
  expect((await snapshot(page)).transportFailures).toEqual([]);
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(1);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );

  await command(page, "releaseRetiredEnvelopes");
  const restored = await snapshot(page);
  expect(restored.pagehidePersisted).toContain(true);
  expect(restored.pageshowPersisted).toContain(true);
  expect(restored.host.physical_sse_connections).toBe(before.host.physical_sse_connections + 1);
  expect(restored.authorizations.length).toBe(before.authorizations.length + 1);
  expect(last(restored.authorizations)?.position).not.toBeNull();
  expect(restored.retiredEnvelopeAttempts).toBeGreaterThanOrEqual(1);
  expect(restored.forwardedEnvelopes).toBeGreaterThan(before.forwardedEnvelopes);
  expect(restored.runtimeResources).toEqual(before.runtimeResources);
});

test("offline freshness preserves ordinary interaction and shutdown retires bounded resources", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name === "chrome-bfcache",
    "Covered by the persisted lifecycle proof.",
  );
  await waitForHostQuiescent(page);
  await page.goto(`${SCENARIO}&synthetic-lifecycle=true`);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await page.context().setOffline(true);
  await expect.poll(async () => last(await freshnessStates(page))).toBe("offline");
  await page.getByRole("button", { name: "Toggle local details" }).press("Enter");
  await expect(page.getByText("Local details are available")).toBeVisible();
  await page.context().setOffline(false);
  await expect.poll(async () => last((await snapshot(page)).freshnessStates)).toBe("current");
  expect((await snapshot(page)).host.active_physical_transports).toBeLessThanOrEqual(1);
  await command(page, "shutdown");
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
  await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(0);
  expect((await snapshot(page)).runtimeResources).toMatchObject({
    authorization: 0,
    buffer: 0,
    membership: 0,
    queue: 0,
    timer: 0,
    transport: 0,
  });
});

test("a sequence gap activates bounded fallback then replays onto one replacement transport", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name === "chrome-bfcache",
    "Covered by the persisted lifecycle proof.",
  );
  const gapOrigin = "http://127.0.0.1:4176";
  await waitForHostQuiescent(page, gapOrigin);
  const baselineResponse = await page.request.get(`${gapOrigin}/__test/iteration-004/inspection`);
  expect(baselineResponse.ok()).toBe(true);
  const baseline = (await baselineResponse.json()) as { physical_sse_connections: number };
  const reset = await page.request.post(
    `${gapOrigin}/__test/iteration-004/control/async/reset-sequence-gap`,
  );
  expect(reset.status()).toBe(204);
  await page.goto(
    "http://127.0.0.1:4176/scenario/iteration004?features=async&format=esm&transport=sse&lifecycle=true&hybrid=true&controlled-clock=true",
  );
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "degraded",
  );
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
  await expect
    .poll(async () => (await snapshot(page)).controlledTimerDelays)
    .toEqual(expect.arrayContaining([125, 10_000, 30_000]));
  await command(page, "advanceGapReauthorization");
  await expect.poll(async () => (await snapshot(page)).authorizations.length).toBe(2);
  await expect.poll(async () => (await snapshot(page)).portsCreated).toBe(2);
  await expect.poll(async () => (await snapshot(page)).subscriptionAttempts).toBe(2);
  try {
    await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(1);
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  try {
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(1);
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  const recovered = await snapshot(page);
  expect(recovered.controlledTimerDelays).toEqual([10_000]);
  expect(recovered.host).toMatchObject({
    active_physical_transports: 1,
    logical_memberships: 1,
  });
  expect(recovered.host.physical_sse_connections).toBe(baseline.physical_sse_connections + 2);
  expect(recovered.transportFailures).toEqual([]);
  const initialAuthorization = recovered.authorizations[0];
  expect(initialAuthorization).toBeDefined();
  expect(last(recovered.authorizations)).toMatchObject({
    position: initialAuthorization?.baseline,
    replay: 2,
  });
  expect(recovered.runtimeResources).toMatchObject({
    buffer: 1,
    membership: 0,
    timer: 1,
    transport: 1,
  });
});

test("one document-wide reconnect restores two logical islands on one new transport", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name === "chrome-bfcache",
    "Covered by the persisted lifecycle proof.",
  );
  await waitForHostQuiescent(page);
  await page.goto(`${SCENARIO}&synthetic-lifecycle=true&islands=2`);
  await expect(page.locator("[data-live-stream-state]")).toHaveCount(2);
  try {
    await expect(page.locator("[data-live-stream-state]").nth(0)).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  await expect(page.locator("[data-live-stream-state]").nth(1)).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  try {
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(2);
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  const before = await snapshot(page);
  expect(before.host.active_physical_transports).toBe(1);

  await command(page, "freeze");
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
  await command(page, "resume");
  await expect
    .poll(async () => (await snapshot(page)).authorizations.length)
    .toBe(before.authorizations.length + 2);
  await expect.poll(async () => (await snapshot(page)).portsCreated).toBe(before.portsCreated + 1);
  await expect
    .poll(async () => (await snapshot(page)).subscriptionAttempts)
    .toBe(before.subscriptionAttempts + 2);
  try {
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(2);
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  expect((await snapshot(page)).host.active_physical_transports).toBe(1);
  await command(page, "shutdown");
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
  await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(0);
});
