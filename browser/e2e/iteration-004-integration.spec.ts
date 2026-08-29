import { expect, test, type Page } from "@playwright/test";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";
const CORE_RUNTIME_RESOURCES = Object.freeze({
  authorization: 0,
  buffer: 0,
  controller: 1,
  extension: 2,
  listener: 4,
  membership: 0,
  observer: 1,
  queue: 0,
  scheduler: 0,
  signal: 0,
  timer: 0,
  transition: 0,
  transport: 0,
});
const ASYNC_RUNTIME_RESOURCES = Object.freeze({
  ...CORE_RUNTIME_RESOURCES,
  buffer: 1,
  listener: 5,
  timer: 1,
  transport: 1,
});

function expectedRuntimeResources(projectName: string, asynchronous: boolean) {
  const lifecycleListeners = projectName === "chromium" || projectName === "chrome-bfcache" ? 4 : 2;
  const core = Object.freeze({ ...CORE_RUNTIME_RESOURCES, listener: lifecycleListeners });
  return asynchronous
    ? Object.freeze({ ...ASYNC_RUNTIME_RESOURCES, listener: lifecycleListeners + 1 })
    : core;
}

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
    readonly paused_upload_operations: number;
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
  if (command === "upload/pause-chunk") {
    await probeCommand(page, "pauseNextUpload");
    return;
  }
  if (command === "upload/resume-chunk") {
    await expect.poll(async () => (await snapshot(page)).host.paused_upload_operations).toBe(1);
    await probeCommand(page, "resumePausedUpload");
    return;
  }
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
  const reset = await page.request.post(
    `${origin}/__test/iteration-004/control/upload/reset-creation-window`,
  );
  expect(reset.status()).toBe(204);
}

async function freshRenderHostState(page: Page): Promise<{
  readonly active_uploads: number;
  readonly fresh_render_paused: boolean;
  readonly paused_upload_operations: number;
}> {
  const response = await page.request.get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`);
  expect(response.ok()).toBe(true);
  const value = (await response.json()) as {
    readonly active_uploads: number;
    readonly fresh_render_paused: boolean;
    readonly paused_upload_operations: number;
  };
  return {
    active_uploads: value.active_uploads,
    fresh_render_paused: value.fresh_render_paused,
    paused_upload_operations: value.paused_upload_operations,
  };
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

interface OrdinaryActionResult {
  readonly action: string;
  readonly domain_count: number;
  readonly html: string;
  readonly revision: number;
}

async function runOrdinaryAction(page: Page): Promise<OrdinaryActionResult> {
  return page.evaluate(async () => {
    const response = await fetch("/scenario/iteration004/action", {
      body: "{}",
      headers: {
        Authorization: "Bearer task1-reference-session",
        "Content-Type": "application/json",
      },
      method: "POST",
    });
    if (!response.ok) throw new Error("iteration_004_ordinary_action_failed");
    return response.json() as Promise<OrdinaryActionResult>;
  });
}

async function probeCommand(page: Page, name: string): Promise<void> {
  await page.evaluate(async (commandName) => {
    const probes: readonly unknown[] = [
      Reflect.get(window, "__suprnovaIteration004"),
      Reflect.get(window, "__suprnovaFreshRender"),
    ];
    const probe = probes.find((candidate) => {
      if (
        (typeof candidate !== "object" && typeof candidate !== "function") ||
        candidate === null
      ) {
        return false;
      }
      return typeof Reflect.get(candidate, commandName) === "function";
    });
    const callback: unknown =
      probe === undefined ? null : Reflect.get(probe as object, commandName);
    if (typeof callback !== "function") throw new Error(`iteration_004_${commandName}_missing`);
    await Reflect.apply(callback, probe, []);
  }, name);
}

for (const format of ["esm", "classic"] as const) {
  for (const features of ["core", "uploads", "async", "both"] as const) {
    test(`${format} ${features} composes only the requested production artifacts`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await waitForHostQuiescent(page);
      const optionalAssetRequests: string[] = [];
      page.on("request", (request) => {
        const pathname = new URL(request.url()).pathname;
        if (/^\/suprnova-live\.(?:uploads|async)\.(?:esm|classic)\.js$/u.test(pathname)) {
          optionalAssetRequests.push(pathname);
        }
      });
      await page.goto(scenario(features, format));
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await expect(page.getByRole("heading", { name: "Iteration 004 integration" })).toBeVisible();
      const ready = await snapshot(page);
      expect(ready.errors).toEqual([]);
      const extensionNames =
        features === "both" ? ["uploads", "async"] : features === "core" ? [] : [features];
      expect(optionalAssetRequests).toEqual(
        extensionNames.map((name) => `/suprnova-live.${name}.${format}.js`),
      );
      const expectedResources = expectedRuntimeResources(
        testInfo.project.name,
        features === "async" || features === "both",
      );
      expect(ready.runtimeResources).toEqual(expectedResources);
      expect(ready.featureRegistrations).toEqual(
        features === "both"
          ? ["uploads:registered", "async:registered"]
          : features === "core"
            ? []
            : [`${features}:registered`],
      );

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
        expect((await snapshot(page)).runtimeResources).toEqual(expectedResources);
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
        expect((await snapshot(page)).runtimeResources).toEqual(expectedResources);
      }

      expect((await snapshot(page)).cspViolations).toEqual([]);
    });
  }
}

for (const format of ["esm", "classic"] as const) {
  for (const affected of ["uploads", "async"] as const) {
    for (const artifact of ["missing", "incompatible"] as const) {
      test(`${format} ${affected} ${artifact} leaves the other optional feature operational`, async ({
        page,
      }, testInfo) => {
        test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
        await waitForHostQuiescent(page);
        const optionalAssets: string[] = [];
        const productionSourceRequests: string[] = [];
        page.on("request", (request) => {
          const pathname = new URL(request.url()).pathname;
          if (/^\/suprnova-live\.(?:uploads|async)\.(?:esm|classic)\.js$/u.test(pathname)) {
            optionalAssets.push(pathname);
          }
          if (pathname.includes("/src/") || pathname.endsWith(".ts")) {
            productionSourceRequests.push(pathname);
          }
        });
        const unaffected = affected === "uploads" ? "async" : "uploads";
        await page.goto(
          scenario(
            "both",
            format,
            `&${affected === "uploads" ? "upload" : "async"}-artifact=${artifact}`,
          ),
        );
        await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
        expect(optionalAssets).toEqual([`/suprnova-live.${unaffected}.${format}.js`]);
        expect(productionSourceRequests).toEqual([]);

        const ready = await snapshot(page);
        expect(ready.errors).toEqual([]);
        expect(ready.cspViolations).toEqual([]);
        expect(ready.featureRegistrations).toEqual(
          artifact === "incompatible"
            ? [`${unaffected}:registered`, `${affected}:incompatible`]
            : [`${unaffected}:registered`],
        );

        if (unaffected === "uploads") {
          await selectRealFile(page);
          await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
            "data-live-upload-state",
            "ready",
          );
          expect(await finalizeSelectedUpload(page)).toBe("finalized");
          await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);
        } else {
          await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
            "data-live-stream-state",
            "current",
          );
          const operational = await snapshot(page);
          expect(operational.host.active_physical_transports).toBe(1);
          expect(operational.host.logical_memberships).toBe(1);
        }

        if (affected === "uploads") {
          await page.locator("#iteration-upload").setInputFiles({
            buffer: Buffer.from("native-fallback"),
            mimeType: "text/plain",
            name: "native-fallback.txt",
          });
          expect(
            await page.locator("#iteration-upload").evaluate((input) => {
              return input instanceof HTMLInputElement ? input.files?.length : -1;
            }),
          ).toBe(1);
          await expect(page.locator("#iteration-upload-progress")).not.toHaveAttribute(
            "data-live-upload-state",
            /.+/u,
          );
          expect((await snapshot(page)).host.active_uploads).toBe(0);
        } else {
          await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
            "data-live-stream-state",
            "disconnected",
          );
          const inactive = await snapshot(page);
          expect(inactive.host.active_physical_transports).toBe(0);
          expect(inactive.host.logical_memberships).toBe(0);
        }

        await page.getByText("Native disclosure", { exact: true }).focus();
        await page.keyboard.press("Enter");
        await expect(page.getByText("Native fallback details")).toBeVisible();
        await expect(page.getByText("Native disclosure", { exact: true })).toBeFocused();
        await Promise.all([
          page.waitForURL((url) => url.pathname === "/scenario/iteration004Destination"),
          page.getByRole("button", { name: "Continue ordinarily" }).press("Enter"),
        ]);
        await expect(
          page.getByRole("heading", { name: "Iteration 004 destination" }),
        ).toBeVisible();
        await waitForHostQuiescent(page);
      });
    }
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

test("the real direct provider stores, reports, verifies, completes, and finalizes", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await waitForHostQuiescent(page);
  await page.goto(scenario("core", "esm"));
  const bytes = Buffer.from("direct!!");
  const createdResponse = await page.request.post(`${REFERENCE_ORIGIN}/__live/uploads`, {
    data: {
      content_type: "application/octet-stream",
      expected_bytes: bytes.length,
      field: "evidence",
      filename: "evidence.bin",
      mode: "direct",
    },
    headers: { Authorization: "Bearer task1-reference-session" },
  });
  expect(createdResponse.status()).toBe(201);
  const created = (await createdResponse.json()) as {
    grant: string;
    handle: string;
    instruction: { method: string; maximum_bytes: number };
  };
  expect(created.instruction).toEqual({
    ...created.instruction,
    maximum_bytes: bytes.length,
    method: "PUT",
  });
  const stored = await page.request.put(
    `${REFERENCE_ORIGIN}/__test/iteration-004/direct/${created.handle}/store`,
    {
      data: bytes,
      headers: {
        Authorization: "Bearer task1-reference-session",
        "Content-Type": "application/octet-stream",
      },
    },
  );
  expect(stored.status()).toBe(204);
  const reported = await page.request.post(
    `${REFERENCE_ORIGIN}/__test/iteration-004/direct/${created.handle}/report`,
    {
      data: { grant: created.grant },
      headers: { Authorization: "Bearer task1-reference-session" },
    },
  );
  expect(reported.status()).toBe(200);
  expect(await reported.json()).toMatchObject({
    next_part: 1,
    receipt: { bytes: bytes.length, disposition: "stored", part: 0 },
    received_bytes: bytes.length,
    state: "transferring",
  });
  const completed = await page.request.post(
    `${REFERENCE_ORIGIN}/__live/uploads/${created.handle}/complete`,
    {
      data: { grant: created.grant },
      headers: { Authorization: "Bearer task1-reference-session" },
    },
  );
  expect(completed.status()).toBe(200);
  const completedValue = (await completed.json()) as { revision: number; state: string };
  expect(completedValue.state).toBe("ready");
  const finalized = await page.request.post(
    `${REFERENCE_ORIGIN}/scenario/iteration004/finalize-upload`,
    {
      data: { handle: created.handle, ready_revision: completedValue.revision },
      headers: { Authorization: "Bearer task1-reference-session" },
    },
  );
  expect(finalized.status()).toBe(200);
  expect(await finalized.json()).toMatchObject({ state: "finalized" });
  await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(0);
});

test("a registered ordinary action commits during an active upload and an async outage", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await waitForHostQuiescent(page);
  await page.goto(scenario("both", "esm", "&transport=sse&synthetic-lifecycle=true"));
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  await control(page, "upload/pause-chunk");
  await selectRealFile(page);
  await expect.poll(async () => (await snapshot(page)).host.active_uploads).toBe(1);
  const duringUpload = await runOrdinaryAction(page);
  expect(duringUpload).toMatchObject({ action: "increment" });
  expect(duringUpload.html).toContain(
    `data-live-domain-count="${duringUpload.domain_count.toString()}"`,
  );
  await control(page, "upload/resume-chunk");
  await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
    "data-live-upload-state",
    "ready",
  );
  expect(await finalizeSelectedUpload(page)).toBe("finalized");

  await probeCommand(page, "freeze");
  await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
  const duringOutage = await runOrdinaryAction(page);
  expect(duringOutage.revision).toBe(duringUpload.revision + 1);
  expect(duringOutage.domain_count).toBe(duringUpload.domain_count + 1);
  await probeCommand(page, "resume");
  try {
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
  } catch {
    throw new Error(JSON.stringify(await snapshot(page)));
  }
  expect((await snapshot(page)).host.active_physical_transports).toBe(1);
  expect((await snapshot(page)).host.active_uploads).toBe(0);
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

for (const morph of ["preserve", "replace"] as const) {
  test(`a production fresh render ${morph}s the active upload boundary with exact cleanup`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/referenceFreshRender?upload-morph=${morph}`);
    await expect(page.locator("html")).toHaveAttribute("data-reference-fresh-render-ready", "true");
    await expect
      .poll(async () => (await freshRenderHostState(page)).fresh_render_paused)
      .toBe(true);

    const input = page.locator("#attachment-input");
    await input.setInputFiles({
      buffer: Buffer.from("active-upload-through-production-morph"),
      mimeType: "text/plain",
      name: "active-morph.txt",
    });
    await expect
      .poll(async () => {
        const state = await freshRenderHostState(page);
        return {
          paused: state.paused_upload_operations,
          uploads: state.active_uploads,
        };
      })
      .toEqual({ paused: 1, uploads: 1 });

    const released = await page.request.post(
      `${REFERENCE_ORIGIN}/__test/iteration-004/control/fresh-render/resume`,
    );
    expect(released.status()).toBe(204);
    await expect(page.locator("[data-live-poll-generation]")).toHaveAttribute(
      "data-live-poll-generation",
      "1",
    );

    const continuity = await page.evaluate(() => {
      const evidence: unknown = Reflect.get(window, "__suprnovaFreshRender");
      if (typeof evidence !== "object" || evidence === null) return null;
      const initialUploadInput: unknown = Reflect.get(evidence, "initialUploadInput");
      const initialUploadProgress: unknown = Reflect.get(evidence, "initialUploadProgress");
      const currentUploadInput = document.querySelector("[live\\:upload='attachment']");
      const currentUploadProgress = document.querySelector("[live\\:progress='attachment']");
      return {
        currentFiles:
          currentUploadInput instanceof HTMLInputElement ? currentUploadInput.files?.length : null,
        inputPreserved: initialUploadInput === currentUploadInput,
        oldInputFiles:
          initialUploadInput instanceof HTMLInputElement ? initialUploadInput.files?.length : null,
        progressPreserved: initialUploadProgress === currentUploadProgress,
      };
    });

    if (morph === "preserve") {
      expect(continuity).toEqual({
        currentFiles: 1,
        inputPreserved: true,
        oldInputFiles: 1,
        progressPreserved: true,
      });
      expect(await freshRenderHostState(page)).toEqual({
        active_uploads: 1,
        fresh_render_paused: false,
        paused_upload_operations: 1,
      });
      await probeCommand(page, "resumePausedUpload");
      await expect(page.locator("[live\\:progress='attachment']")).toHaveAttribute(
        "data-live-upload-state",
        "ready",
      );
      expect(
        await page.evaluate(async () => {
          const evidence: unknown = Reflect.get(window, "__suprnovaFreshRender");
          const finalize: unknown =
            typeof evidence === "object" && evidence !== null
              ? Reflect.get(evidence, "finalizeSelectedUpload")
              : null;
          if (typeof finalize !== "function") throw new Error("fresh_render_finalize_missing");
          return Reflect.apply(finalize, evidence, []) as Promise<string>;
        }),
      ).toBe("finalized");
    } else {
      expect(continuity).toEqual({
        currentFiles: 0,
        inputPreserved: false,
        oldInputFiles: 0,
        progressPreserved: false,
      });
    }
    await expect
      .poll(async () => {
        const state = await freshRenderHostState(page);
        return {
          paused: state.paused_upload_operations,
          uploads: state.active_uploads,
        };
      })
      .toEqual({ paused: 0, uploads: 0 });
  });
}
