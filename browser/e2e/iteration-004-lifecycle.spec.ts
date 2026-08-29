import { expect, test, type Page } from "@playwright/test";

test.use({ trace: "off" });

const SCENARIO =
  "http://127.0.0.1:4175/scenario/iteration004?features=async&format=esm&transport=sse&lifecycle=true&hybrid=true";
const BOTH_SCENARIO = SCENARIO.replace("features=async", "features=both");

interface LifecycleSnapshot {
  readonly authorizations: readonly Readonly<{
    readonly baseline: Readonly<{ readonly epoch: string; readonly sequence: string }>;
    readonly position: Readonly<{ readonly epoch: string; readonly sequence: string }> | null;
    readonly replay: number;
    readonly requestedGeneration: number;
    readonly serverGeneration: number;
    readonly subscription: string;
    readonly transport: string;
  }>[];
  readonly controlledTimerDelays: readonly number[];
  readonly forwardedEnvelopes: number;
  readonly freshnessStates: readonly string[];
  readonly heldEnvelopes: number;
  readonly host: Readonly<{
    readonly active_physical_transports: number;
    readonly active_uploads: number;
    readonly logical_memberships: number;
    readonly open_timers: number;
    readonly paused_upload_operations: number;
    readonly physical_sse_connections: number;
    readonly physical_websocket_connections: number;
  }>;
  readonly pagehidePersisted: readonly boolean[];
  readonly pageshowPersisted: readonly boolean[];
  readonly portsCreated: number;
  readonly retiredEnvelopeAttempts: number;
  readonly subscriptionAttempts: number;
  readonly successorAcknowledgmentsHeld: number;
  readonly successorBarriersInstalled: number;
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

async function selectPausedUpload(page: Page, name: string): Promise<void> {
  await command(page, "pauseNextUpload");
  await page.locator("#iteration-upload").setInputFiles({
    buffer: Buffer.from(`active-${name}`),
    mimeType: "text/plain",
    name: `${name}.txt`,
  });
  await expect
    .poll(async () => {
      const current = await snapshot(page);
      return {
        paused: current.host.paused_upload_operations,
        uploads: current.host.active_uploads,
      };
    })
    .toEqual({ paused: 1, uploads: 1 });
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
  const reset = await page.request.post(
    `${origin}/__test/iteration-004/control/upload/reset-creation-window`,
  );
  expect(reset.status()).toBe(204);
}

for (const bfcacheTransport of ["sse", "websocket"] as const) {
  test(`real bfcache restoration retires the old ${bfcacheTransport} generation and reauthorizes from current position`, async ({
    page,
  }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chrome-bfcache",
      "Persisted PageTransitionEvent proof runs in dedicated stable Chrome.",
    );
    await waitForHostQuiescent(page);
    await page.goto(
      `${BOTH_SCENARIO.replace("transport=sse", `transport=${bfcacheTransport}`)}&controlled-clock=true`,
    );
    try {
      await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );
    } catch {
      throw new Error(JSON.stringify(await snapshot(page)));
    }
    await expect.poll(async () => (await snapshot(page)).forwardedEnvelopes).toBe(1);
    await selectPausedUpload(page, `bfcache-${bfcacheTransport}`);
    const beforeHold = await snapshot(page);
    await command(page, "holdNextEnvelope");
    await command(page, "emitNextEnvelope");
    await expect.poll(async () => (await snapshot(page)).heldEnvelopes).toBe(1);
    expect((await snapshot(page)).forwardedEnvelopes).toBe(beforeHold.forwardedEnvelopes);
    await command(page, "holdSuccessorDelivery");
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
        if (!response.ok()) return null;
        const value = (await response.json()) as {
          active_physical_transports: number;
          active_uploads: number;
          logical_memberships: number;
          open_timers: number;
          paused_upload_operations: number;
        };
        return {
          memberships: value.logical_memberships,
          physical: value.active_physical_transports,
          pausedUploads: value.paused_upload_operations,
          timers: value.open_timers,
          uploads: value.active_uploads,
        };
      })
      .toEqual({ memberships: 0, physical: 0, pausedUploads: 0, timers: 0, uploads: 1 });
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
    try {
      await expect
        .poll(async () => (await snapshot(page)).portsCreated)
        .toBe(before.portsCreated + 1);
    } catch {
      throw new Error(JSON.stringify(await snapshot(page)));
    }
    await expect
      .poll(async () => (await snapshot(page)).subscriptionAttempts)
      .toBe(before.subscriptionAttempts + 1);
    expect((await snapshot(page)).transportFailures).toEqual([]);
    await expect
      .poll(async () => {
        const current = await snapshot(page);
        return {
          acknowledgments: current.successorAcknowledgmentsHeld,
          barriers: current.successorBarriersInstalled,
          forwarded: current.forwardedEnvelopes,
        };
      })
      .toEqual({ acknowledgments: 1, barriers: 1, forwarded: before.forwardedEnvelopes });
    await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(1);
    const beforeRelease = await snapshot(page);
    const heldSuccessorPosition = last(beforeRelease.authorizations)?.position;
    expect(heldSuccessorPosition).not.toBeNull();
    const beforeReleaseDom = await page
      .locator("[data-suprnova-live-island]")
      .evaluate((node) => node.outerHTML);
    const beforeReleasePresentation = await page
      .locator("[data-live-stream-state]")
      .getAttribute("data-live-stream-state");
    await command(page, "releaseRetiredEnvelopes");
    await expect.poll(async () => (await snapshot(page)).retiredEnvelopeAttempts).toBe(1);
    const restored = await snapshot(page);
    expect(restored.pagehidePersisted).toContain(true);
    expect(restored.pageshowPersisted).toContain(true);
    const connectionKey =
      bfcacheTransport === "sse" ? "physical_sse_connections" : "physical_websocket_connections";
    expect(restored.host[connectionKey]).toBe(before.host[connectionKey] + 1);
    expect(restored.authorizations.length).toBe(before.authorizations.length + 1);
    expect(last(restored.authorizations)?.position).not.toBeNull();
    expect(restored.authorizations[0]?.requestedGeneration).toBe(
      restored.authorizations[0]?.serverGeneration,
    );
    expect(last(restored.authorizations)?.requestedGeneration).toBe(
      last(restored.authorizations)?.serverGeneration,
    );
    expect(last(restored.authorizations)?.transport).not.toBe(
      restored.authorizations[0]?.transport,
    );
    expect(restored.forwardedEnvelopes).toBe(beforeRelease.forwardedEnvelopes);
    expect(restored.freshnessStates).toEqual(beforeRelease.freshnessStates);
    expect(restored.authorizations).toEqual(beforeRelease.authorizations);
    expect(restored.host).toEqual(beforeRelease.host);
    expect(restored.runtimeResources).toEqual(beforeRelease.runtimeResources);
    expect(
      await page.locator("[data-suprnova-live-island]").evaluate((node) => node.outerHTML),
    ).toBe(beforeReleaseDom);
    expect(
      await page.locator("[data-live-stream-state]").getAttribute("data-live-stream-state"),
    ).toBe(beforeReleasePresentation);
    expect(beforeReleasePresentation).toBe("connecting");

    await command(page, "replaceHeldSuccessorAndHoldNext");
    await expect
      .poll(async () => (await snapshot(page)).authorizations.length)
      .toBe(before.authorizations.length + 2);
    await expect
      .poll(async () => {
        const current = await snapshot(page);
        return {
          acknowledgments: current.successorAcknowledgmentsHeld,
          barriers: current.successorBarriersInstalled,
        };
      })
      .toEqual({ acknowledgments: 2, barriers: 2 });
    const staleGenerationProbe = await snapshot(page);
    expect(last(staleGenerationProbe.authorizations)?.position).toEqual(heldSuccessorPosition);
    expect(last(staleGenerationProbe.authorizations)?.replay).toBe(2);

    await command(page, "releaseSuccessorDelivery");
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
    await expect
      .poll(async () => (await snapshot(page)).forwardedEnvelopes)
      .toBe(before.forwardedEnvelopes + 1);
    await command(page, "replaceCurrentTransport");
    await expect
      .poll(async () => (await snapshot(page)).authorizations.length)
      .toBe(before.authorizations.length + 3);
    const replayed = await snapshot(page);
    expect(last(replayed.authorizations)?.position).toEqual({
      epoch: heldSuccessorPosition?.epoch,
      sequence: String(BigInt(heldSuccessorPosition?.sequence ?? "0") + 3n),
    });
    expect(last(replayed.authorizations)?.replay).toBe(0);
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
    await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(1);
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(1);
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "interrupted",
    );
    expect(
      await page.locator("#iteration-upload").evaluate((input) => {
        return input instanceof HTMLInputElement ? input.files?.length : null;
      }),
    ).toBe(1);
    await page.getByRole("button", { name: "Retry upload" }).click();
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "ready",
    );
    await command(page, "finalizeSelectedUpload");
    await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);
  });
}

test("ordinary navigation cancels an active upload and retires every document resource", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name === "chrome-bfcache",
    "Persisted navigation has a dedicated proof.",
  );
  await waitForHostQuiescent(page);
  await page.goto(BOTH_SCENARIO);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await selectPausedUpload(page, "navigation");

  await page.getByRole("link", { name: "Ordinary destination" }).click();
  await expect(page.getByRole("heading", { name: "Iteration 004 destination" })).toBeVisible();
  await expect
    .poll(async () => {
      const response = await page.request.get(
        "http://127.0.0.1:4175/__test/iteration-004/inspection",
      );
      const value = (await response.json()) as LifecycleSnapshot["host"];
      return {
        memberships: value.logical_memberships,
        physical: value.active_physical_transports,
        pausedUploads: value.paused_upload_operations,
        timers: value.open_timers,
        uploads: value.active_uploads,
      };
    })
    .toEqual({ memberships: 0, physical: 0, pausedUploads: 0, timers: 0, uploads: 0 });
});

test("freeze and resume preserve active upload retry authority while shutdown cancels it", async ({
  page,
}, testInfo) => {
  test.skip(
    testInfo.project.name === "chrome-bfcache",
    "Persisted lifecycle has a dedicated proof.",
  );
  await waitForHostQuiescent(page);
  await page.goto(`${BOTH_SCENARIO}&synthetic-lifecycle=true`);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await selectPausedUpload(page, "freeze-resume");

  await command(page, "freeze");
  await expect
    .poll(async () => {
      const current = await snapshot(page);
      return {
        physical: current.host.active_physical_transports,
        pausedUploads: current.host.paused_upload_operations,
        uploads: current.host.active_uploads,
      };
    })
    .toEqual({ physical: 0, pausedUploads: 0, uploads: 1 });
  await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
    "data-live-upload-state",
    "interrupted",
  );
  expect(
    await page.locator("#iteration-upload").evaluate((input) => {
      return input instanceof HTMLInputElement ? input.files?.length : null;
    }),
  ).toBe(1);

  await command(page, "resume");
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await page.getByRole("button", { name: "Retry upload" }).click();
  await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
    "data-live-upload-state",
    "ready",
  );
  await command(page, "finalizeSelectedUpload");
  await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);

  await selectPausedUpload(page, "shutdown");
  await command(page, "shutdown");
  await expect
    .poll(async () => {
      const current = await snapshot(page);
      return {
        memberships: current.host.logical_memberships,
        physical: current.host.active_physical_transports,
        pausedUploads: current.host.paused_upload_operations,
        uploads: current.host.active_uploads,
      };
    })
    .toEqual({ memberships: 0, physical: 0, pausedUploads: 0, uploads: 0 });
  expect(
    await page.locator("#iteration-upload").evaluate((input) => {
      return input instanceof HTMLInputElement ? input.files?.length : null;
    }),
  ).toBe(0);
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
  expect((await snapshot(page)).host.active_physical_transports).toBe(1);
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
  await command(page, "advanceTransportReconnect");
  await expect.poll(async () => (await snapshot(page)).authorizations.length).toBe(2);
  try {
    await expect.poll(async () => (await snapshot(page)).portsCreated).toBe(2);
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
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
