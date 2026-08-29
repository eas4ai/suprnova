import { expect, test, type Page } from "@playwright/test";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";

interface IntegrationSnapshot {
  readonly authorizations: readonly Readonly<{
    readonly position: Readonly<{ readonly epoch: string; readonly sequence: string }> | null;
    readonly subscription: string;
    readonly transport: string;
  }>[];
  readonly cspViolations: readonly string[];
  readonly errors: readonly string[];
  readonly featureRegistrations: readonly string[];
  readonly host: Readonly<{
    readonly active_physical_transports: number;
    readonly active_uploads: number;
    readonly logical_memberships: number;
    readonly physical_sse_connections: number;
    readonly physical_websocket_connections: number;
  }>;
  readonly runtimeResources: Readonly<Record<string, number>>;
}

function scenario(
  features: "async" | "both" | "core" | "uploads",
  format: "classic" | "esm",
  extra = "",
): string {
  return `${REFERENCE_ORIGIN}/scenario/iteration004?features=${features}&format=${format}${extra}`;
}

async function snapshot(page: Page): Promise<IntegrationSnapshot> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    if ((typeof probe !== "object" && typeof probe !== "function") || probe === null) {
      throw new Error("iteration_004_probe_missing");
    }
    const inspect: unknown = Reflect.get(probe, "snapshot");
    if (typeof inspect !== "function") throw new Error("iteration_004_snapshot_missing");
    return Reflect.apply(inspect, probe, []) as IntegrationSnapshot;
  });
}

async function control(page: Page, command: string): Promise<void> {
  await page.evaluate(async (path) => {
    const response = await fetch(`/__test/iteration-004/control/${path}`, { method: "POST" });
    if (!response.ok) throw new Error(`reference_control_${path}_failed`);
  }, command);
}

async function waitForHostQuiescent(page: Page, origin = REFERENCE_ORIGIN): Promise<void> {
  await expect
    .poll(async () => {
      const response = await page.request.get(`${origin}/__test/iteration-004/inspection`);
      if (!response.ok()) return null;
      const value = (await response.json()) as {
        active_physical_transports: number;
        active_uploads: number;
        logical_memberships: number;
      };
      return {
        physical: value.active_physical_transports,
        uploads: value.active_uploads,
        memberships: value.logical_memberships,
      };
    })
    .toEqual({ physical: 0, uploads: 0, memberships: 0 });
}

async function selectRealFile(page: Page): Promise<void> {
  await page.locator("#iteration-upload").setInputFiles({
    buffer: Buffer.from("iteration-004-production-upload"),
    mimeType: "text/plain",
    name: "iteration-004.txt",
  });
}

async function finalizeSelectedUpload(page: Page): Promise<string> {
  return page.evaluate(async () => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    if ((typeof probe !== "object" && typeof probe !== "function") || probe === null) {
      throw new Error("iteration_004_probe_missing");
    }
    const finalize: unknown = Reflect.get(probe, "finalizeSelectedUpload");
    if (typeof finalize !== "function") throw new Error("iteration_004_finalize_missing");
    return Reflect.apply(finalize, probe, []) as Promise<string>;
  });
}

for (const format of ["esm", "classic"] as const) {
  for (const features of ["core", "uploads", "async", "both"] as const) {
    test(`${format} ${features} composes only the requested production artifacts`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await waitForHostQuiescent(page);
      await page.goto(scenario(features, format));
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await expect(page.getByRole("heading", { name: "Iteration 004 integration" })).toBeVisible();
      const ready = await snapshot(page);
      expect(ready.errors).toEqual([]);
      if (format === "esm" && features !== "core") {
        expect(ready.featureRegistrations).toEqual(
          features === "both"
            ? ["uploads:registered", "async:registered"]
            : [`${features}:registered`],
        );
      }

      if (features === "core") {
        await page.getByRole("button", { name: "Toggle local details" }).press("Enter");
        await expect(page.getByText("Local details are available")).toBeVisible();
      }

      if (features === "uploads" || features === "both") {
        await control(page, "upload/pause-chunk");
        await selectRealFile(page);
        await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
          "data-live-upload-state",
          "transferring",
        );
        await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(1);
        expect((await snapshot(page)).runtimeResources["extension"]).toBeGreaterThanOrEqual(1);
        await control(page, "upload/resume-chunk");
        await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
          "data-live-upload-state",
          "ready",
        );
        expect(await finalizeSelectedUpload(page)).toBe("finalized");
        await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);
      }

      if (features === "async" || features === "both") {
        expect((await snapshot(page)).errors).toEqual([]);
        await expect(page.locator("[data-live-stream-state]").first()).toHaveAttribute(
          "data-live-stream-state",
          "current",
        );
        await expect
          .poll(async () => (await snapshot(page)).host.active_physical_transports)
          .toBe(1);
        await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(1);
        expect((await snapshot(page)).runtimeResources).toMatchObject({
          buffer: 1,
          transport: 1,
        });
      }

      expect((await snapshot(page)).cspViolations).toEqual([]);
    });
  }
}

test("the both-feature page transfers one native file beside one current logical stream", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await waitForHostQuiescent(page);
  await page.goto(scenario("both", "esm", "&transport=sse"));
  await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );

  const before = await snapshot(page);
  await control(page, "upload/pause-chunk");
  await selectRealFile(page);
  await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
    "data-live-upload-state",
    "transferring",
  );
  await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(1);
  const during = await snapshot(page);
  expect(during.host).toMatchObject({
    active_physical_transports: 1,
    active_uploads: 1,
    logical_memberships: 1,
  });
  expect(during.host.physical_sse_connections - before.host.physical_sse_connections).toBe(0);

  await page.getByRole("button", { name: "Toggle local details" }).press("Enter");
  await expect(page.getByText("Local details are available")).toBeVisible();
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );

  await control(page, "upload/resume-chunk");
  await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
    "data-live-upload-state",
    "ready",
  );
  expect(await finalizeSelectedUpload(page)).toBe("finalized");
  await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
});

test("two islands share one physical SSE transport and removing one membership keeps it open", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await waitForHostQuiescent(page);
  await page.goto(scenario("async", "esm", "&islands=2&transport=sse"));
  await expect(page.locator("[data-live-stream-state]")).toHaveCount(2);
  await expect(page.locator("[data-live-stream-state]").nth(0)).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await expect(page.locator("[data-live-stream-state]").nth(1)).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(2);
  expect((await snapshot(page)).host.active_physical_transports).toBe(1);

  await page.getByRole("button", { name: "Remove second island" }).click();
  await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(1);
  expect((await snapshot(page)).host.active_physical_transports).toBe(1);
});

test("production WebSocket and Rust polling routes remain physical and ordinary navigation remains HTTP", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await waitForHostQuiescent(page);
  const beforeWebSocket = await page.request
    .get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`)
    .then(async (response) => {
      expect(response.ok()).toBe(true);
      return (await response.json()) as {
        physical_websocket_connections: number;
      };
    });
  await page.goto(scenario("async", "esm", "&transport=websocket"));
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await expect
    .poll(async () => (await snapshot(page)).host.physical_websocket_connections)
    .toBe(beforeWebSocket.physical_websocket_connections + 1);
  expect((await snapshot(page)).host).toMatchObject({
    active_physical_transports: 1,
    logical_memberships: 1,
  });

  await page.getByRole("link", { name: "Ordinary destination" }).click();
  await expect(page.getByRole("heading", { name: "Iteration 004 destination" })).toBeVisible();
  expect(new URL(page.url()).origin).toBe(REFERENCE_ORIGIN);
  await waitForHostQuiescent(page);

  await page.goto(`${REFERENCE_ORIGIN}/scenario/referenceFreshRender`);
  await expect(page.locator("[data-live-poll-generation]")).toHaveAttribute(
    "data-live-poll-generation",
    "1",
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const evidence: unknown = Reflect.get(window, "__suprnovaFreshRender");
        if (typeof evidence !== "object" || evidence === null) return null;
        const acceptedRevision: unknown = Reflect.get(evidence, "acceptedRevision");
        const requests: unknown = Reflect.get(evidence, "requests");
        return {
          acceptedRevision,
          requests,
        };
      }),
    )
    .toEqual({ acceptedRevision: "1", requests: 1 });
  expect(
    await page.evaluate(() => {
      const evidence: unknown = Reflect.get(window, "__suprnovaFreshRender");
      if (typeof evidence !== "object" || evidence === null) return null;
      const currentIsland = document.querySelector("[data-suprnova-live-island]");
      const currentPreserved = document.querySelector("#fresh-render-preserved");
      const currentReplacement = document.querySelector("#fresh-render-replacement");
      const initialIsland: unknown = Reflect.get(evidence, "initialIsland");
      const initialPreserved: unknown = Reflect.get(evidence, "initialPreserved");
      const initialReplacement: unknown = Reflect.get(evidence, "initialReplacement");
      return {
        focusPreserved: document.activeElement === currentPreserved,
        islandPreserved: initialIsland === currentIsland,
        nodeReplaced: initialReplacement !== currentReplacement,
        preservedControl: initialPreserved === currentPreserved,
        replacementTag: currentReplacement?.tagName ?? null,
      };
    }),
  ).toEqual({
    focusPreserved: true,
    islandPreserved: true,
    nodeReplaced: true,
    preservedControl: true,
    replacementTag: "ARTICLE",
  });
});
