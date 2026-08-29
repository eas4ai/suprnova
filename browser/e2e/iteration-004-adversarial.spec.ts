import { createHash } from "node:crypto";
import { connect } from "node:net";

import { expect, test, type APIResponse, type Page } from "@playwright/test";

import { forgeCanonicalGrantSignature } from "./support/grant-mutation.js";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";
const AUTHORIZATION = "Bearer task1-reference-session";

interface Snapshot {
  readonly authorizations: readonly unknown[];
  readonly cspViolations: readonly string[];
  readonly errors: readonly string[];
  readonly heldEnvelopes: number;
  readonly host: Readonly<{
    readonly active_physical_transports: number;
    readonly active_uploads: number;
    readonly logical_memberships: number;
    readonly physical_sse_connections: number;
    readonly physical_websocket_connections: number;
    readonly validation_scan_calls: number;
  }>;
  readonly retiredEnvelopeAttempts: number;
  readonly transportFailures: readonly string[];
}

interface CreatedUpload {
  readonly grant: string;
  readonly handle: string;
}

interface OrdinaryActionResult {
  readonly domain_count: number;
  readonly revision: number;
}

interface AsyncAdversarialOutcome {
  readonly accepted_sequence: number;
  readonly ceiling_bytes: number;
  readonly ceiling_events: number;
  readonly dependent_closed: boolean;
  readonly disposition: string;
  readonly recovery: string;
  readonly retained_bytes: number;
  readonly retained_events: number;
  readonly sibling_usable: boolean;
  readonly wire: string | null;
}

interface UploadRaceOutcome {
  readonly accepted_outcomes: number;
  readonly active_uploads: number;
  readonly disposition: string;
  readonly terminal_state: string;
}

interface RawUpgradeResponse {
  readonly body: string;
  readonly status: number;
}

interface InterceptedUploadFailure {
  readonly body: string;
  readonly grant: string;
  readonly status: number;
}

function scenario(extra = ""): string {
  return `${REFERENCE_ORIGIN}/scenario/iteration004?features=both&format=esm&transport=sse&synthetic-lifecycle=true${extra}`;
}

async function snapshot(page: Page): Promise<Snapshot> {
  return page.evaluate(() => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    if ((typeof probe !== "object" && typeof probe !== "function") || probe === null) {
      throw new Error("iteration_004_probe_missing");
    }
    const inspect: unknown = Reflect.get(probe, "snapshot");
    if (typeof inspect !== "function") throw new Error("iteration_004_snapshot_missing");
    return Reflect.apply(inspect, probe, []) as Promise<Snapshot>;
  });
}

async function command(page: Page, name: string): Promise<void> {
  await page.evaluate(async (commandName) => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    const callback: unknown =
      (typeof probe === "object" || typeof probe === "function") && probe !== null
        ? Reflect.get(probe, commandName)
        : null;
    if (typeof callback !== "function") throw new Error(`iteration_004_${commandName}_missing`);
    await Reflect.apply(callback, probe, []);
  }, name);
}

async function commandResult<T>(page: Page, name: string): Promise<T> {
  return page.evaluate(async (commandName) => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    const callback: unknown =
      (typeof probe === "object" || typeof probe === "function") && probe !== null
        ? Reflect.get(probe, commandName)
        : null;
    if (typeof callback !== "function") throw new Error(`iteration_004_${commandName}_missing`);
    return Reflect.apply(callback, probe, []) as Promise<T>;
  }, name);
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

async function createUpload(page: Page, expectedBytes: number): Promise<CreatedUpload> {
  const response = await page.request.post(`${REFERENCE_ORIGIN}/__live/uploads`, {
    data: {
      content_type: "application/octet-stream",
      expected_bytes: expectedBytes,
      field: "evidence",
      filename: "adversarial.bin",
      mode: "file",
    },
    headers: { Authorization: AUTHORIZATION },
  });
  expect(response.status()).toBe(201);
  return response.json() as Promise<CreatedUpload>;
}

async function assertClosedResponse(
  response: APIResponse,
  expectedStatus: number,
  expectedCode: string,
  sentinels: readonly string[],
): Promise<void> {
  const body = await response.text();
  expect(Buffer.byteLength(body)).toBeLessThanOrEqual(4_096);
  expect(JSON.parse(body)).toEqual({ error: expectedCode });
  expect(response.status()).toBe(expectedStatus);
  for (const sentinel of sentinels) expect(body).not.toContain(sentinel);
}

async function waitForHostQuiescent(page: Page): Promise<void> {
  await expect
    .poll(async () => {
      const response = await page.request.get(
        `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
      );
      if (!response.ok()) return null;
      const value = (await response.json()) as {
        active_physical_transports: number;
        logical_memberships: number;
      };
      return {
        memberships: value.logical_memberships,
        physical: value.active_physical_transports,
      };
    })
    .toEqual({ memberships: 0, physical: 0 });
  await expect
    .poll(async () => {
      const reset = await page.request.post(
        `${REFERENCE_ORIGIN}/__test/iteration-004/control/upload/reset-creation-window`,
      );
      return reset.status();
    })
    .toBe(204);
}

async function rawWebSocketUpgrade(headers: readonly string[]): Promise<RawUpgradeResponse> {
  return new Promise((resolve, reject) => {
    const socket = connect({ host: "127.0.0.1", port: 4175 });
    const chunks: Buffer[] = [];
    let settled = false;
    socket.once("error", (error) => {
      if (!settled) reject(error);
    });
    socket.on("data", (chunk: Buffer) => {
      chunks.push(chunk);
      const response = Buffer.concat(chunks).toString("utf8");
      const boundary = response.indexOf("\r\n\r\n");
      if (boundary < 0) return;
      const head = response.slice(0, boundary);
      const body = response.slice(boundary + 4);
      const match = /^HTTP\/1\.1 (\d{3})/u.exec(head);
      if (match === null) {
        settled = true;
        socket.destroy();
        reject(new Error(`invalid websocket upgrade response: ${response}`));
        return;
      }
      const contentLengthMatch = /^content-length:\s*(\d+)\s*$/imu.exec(head);
      const contentLength = contentLengthMatch === null ? 0 : Number(contentLengthMatch[1]);
      if (Buffer.byteLength(body) < contentLength) return;
      settled = true;
      socket.destroy();
      resolve({ body, status: Number(match[1]) });
    });
    socket.once("connect", () => {
      socket.write(
        [
          "GET /__live/async/ws HTTP/1.1",
          "Host: 127.0.0.1:4175",
          "Upgrade: websocket",
          "Connection: Upgrade",
          "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
          "Sec-WebSocket-Version: 13",
          "X-Live-Transport: unknown-transport",
          ...headers,
          "",
          "",
        ].join("\r\n"),
      );
    });
  });
}

async function assertNoGrantLeak(page: Page, sentinels: readonly string[]): Promise<void> {
  const observable = await page.evaluate(async () => {
    const databases = typeof indexedDB.databases === "function" ? await indexedDB.databases() : [];
    const storage = (area: Storage) =>
      Array.from({ length: area.length }, (_, index) => {
        const key = area.key(index);
        return key === null ? null : [key, area.getItem(key)];
      });
    return [
      document.documentElement.outerHTML,
      document.body.innerText,
      document.URL,
      location.href,
      JSON.stringify(history.state),
      document.cookie,
      JSON.stringify(storage(localStorage)),
      JSON.stringify(storage(sessionStorage)),
      JSON.stringify(databases),
      JSON.stringify(performance.getEntriesByType("resource").map((entry) => entry.name)),
      document.querySelector("[role='alert']")?.textContent ?? "",
    ].join("\n");
  });
  const inspection = await page.request.get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`);
  expect(inspection.status()).toBe(200);
  const inspectionText = await inspection.text();
  for (const sentinel of sentinels) {
    expect(observable).not.toContain(sentinel);
    expect(inspectionText).not.toContain(sentinel);
  }
}

async function resetUploadCreationWindow(page: Page): Promise<void> {
  await expect
    .poll(async () => {
      const inspection = await page.request.get(
        `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
      );
      if (!inspection.ok()) return null;
      return ((await inspection.json()) as { readonly active_uploads: number }).active_uploads;
    })
    .toBe(0);
  const reset = await page.request.post(
    `${REFERENCE_ORIGIN}/__test/iteration-004/control/upload/reset-creation-window`,
  );
  expect(reset.status()).toBe(204);
}

test("hostile upload authority and chunk shapes fail closed without poisoning ordinary actions", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=esm`);
  await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
  const created = await createUpload(page, 262_146);
  const uploads = [created];
  const foreignHandle = "018f47c1-2af0-7cc4-a001-000000000099";
  const checksum = createHash("sha256").update(Buffer.alloc(8, 0x61)).digest("hex");
  const sentinels = [created.grant, AUTHORIZATION];
  let previousAction = await runOrdinaryAction(page);

  const cases: readonly Readonly<{
    execute(): Promise<APIResponse>;
    code: string;
    status: number;
  }>[] = [
    {
      code: "upload_conflict",
      execute: () =>
        page.request.get(`${REFERENCE_ORIGIN}/__live/uploads/${foreignHandle}`, {
          headers: { Authorization: AUTHORIZATION, "X-Live-Upload-Grant": created.grant },
        }),
      status: 409,
    },
    {
      code: "input_too_large",
      execute: () =>
        page.request.post(`${REFERENCE_ORIGIN}/__live/uploads/${created.handle}/chunks/0`, {
          data: Buffer.alloc(262_145, 0x61),
          headers: {
            Authorization: AUTHORIZATION,
            "Content-Type": "application/octet-stream",
            "X-Live-Chunk-Bytes": "262145",
            "X-Live-Chunk-Sha256": createHash("sha256")
              .update(Buffer.alloc(262_145, 0x61))
              .digest("hex"),
            "X-Live-Upload-Grant": created.grant,
          },
        }),
      status: 413,
    },
    {
      code: "upload_conflict",
      execute: () =>
        page.request.post(`${REFERENCE_ORIGIN}/__live/uploads/${created.handle}/chunks/1`, {
          data: Buffer.alloc(8, 0x61),
          headers: {
            Authorization: AUTHORIZATION,
            "Content-Type": "application/octet-stream",
            "X-Live-Chunk-Bytes": "8",
            "X-Live-Chunk-Sha256": checksum,
            "X-Live-Upload-Grant": created.grant,
          },
        }),
      status: 409,
    },
    {
      code: "upload_incomplete_transfer",
      execute: async () => {
        const truncated = await createUpload(page, 8);
        uploads.push(truncated);
        sentinels.push(truncated.grant);
        return page.request.post(
          `${REFERENCE_ORIGIN}/__live/uploads/${truncated.handle}/chunks/0`,
          {
            data: Buffer.alloc(4, 0x61),
            headers: {
              Authorization: AUTHORIZATION,
              "Content-Type": "application/octet-stream",
              "X-Live-Chunk-Bytes": "8",
              "X-Live-Chunk-Sha256": checksum,
              "X-Live-Upload-Grant": truncated.grant,
            },
          },
        );
      },
      status: 409,
    },
  ];

  for (const attack of cases) {
    await assertClosedResponse(await attack.execute(), attack.status, attack.code, sentinels);
    const nextAction = await runOrdinaryAction(page);
    expect(nextAction.revision).toBe(previousAction.revision + 1);
    expect(nextAction.domain_count).toBe(previousAction.domain_count + 1);
    previousAction = nextAction;
  }

  const validStatus = await page.request.get(
    `${REFERENCE_ORIGIN}/__live/uploads/${created.handle}`,
    { headers: { Authorization: AUTHORIZATION, "X-Live-Upload-Grant": created.grant } },
  );
  expect(validStatus.status()).toBe(200);
  const inspected = await page.evaluate(() => {
    const storage: string[] = [];
    for (const area of [localStorage, sessionStorage]) {
      for (let index = 0; index < area.length; index += 1) {
        const key = area.key(index);
        if (key !== null) storage.push(area.getItem(key) ?? "");
      }
    }
    return {
      dom: document.documentElement.outerHTML,
      resources: performance.getEntriesByType("resource").map((entry) => entry.name),
      storage,
      url: location.href,
    };
  });
  for (const value of [
    ...inspected.resources,
    ...inspected.storage,
    inspected.dom,
    inspected.url,
  ]) {
    expect(value).not.toContain(created.grant);
  }
  expect((await snapshot(page)).cspViolations).toEqual([]);
  for (const upload of uploads) {
    const canceled = await page.request.post(
      `${REFERENCE_ORIGIN}/__live/uploads/${upload.handle}/cancel`,
      {
        headers: {
          Authorization: AUTHORIZATION,
          "X-Live-Upload-Grant": upload.grant,
        },
      },
    );
    expect(canceled.status()).toBe(200);
  }
});

for (const format of ["esm", "classic"] as const) {
  test(`${format} production upload runtime keeps every hostile grant secret and retryable`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    const consoleDiagnostics: string[] = [];
    const pageDiagnostics: string[] = [];
    page.on("console", (message) => consoleDiagnostics.push(message.text()));
    page.on("pageerror", (error) => pageDiagnostics.push(error.message));
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");

    const cases = [
      ["forged-grant", 401, "invalid_transfer_grant"],
      ["cross-scope-handle", 409, "upload_conflict"],
      ["oversized-chunk", 413, "input_too_large"],
      ["truncated-chunk", 409, "upload_incomplete_transfer"],
      ["reordered-chunk", 409, "upload_conflict"],
    ] as const;
    const sentinels: string[] = [];
    let previousAction = await runOrdinaryAction(page);

    for (const [name, expectedStatus, expectedCode] of cases) {
      const repetitions = name === "forged-grant" ? 20 : 1;
      for (let repetition = 0; repetition < repetitions; repetition += 1) {
        let intercepted: InterceptedUploadFailure | null = null;
        const routePattern = "**/__live/uploads/**/chunks/**";
        await page.route(routePattern, async (route, request) => {
          if (intercepted !== null) {
            await route.continue();
            return;
          }
          const originalHeaders = request.headers();
          const grant = originalHeaders["x-live-upload-grant"];
          if (grant === undefined || grant.length === 0) {
            await route.abort("failed");
            throw new Error("production_upload_grant_not_observed");
          }
          const headers = { ...originalHeaders };
          let body = request.postDataBuffer() ?? Buffer.alloc(0);
          let url = request.url();
          if (name === "forged-grant") {
            headers["x-live-upload-grant"] = forgeCanonicalGrantSignature(grant);
          } else if (name === "cross-scope-handle") {
            url = url.replace(
              /\/uploads\/[^/]+\/chunks\//u,
              "/uploads/018f47c1-2af0-7cc4-a001-000000000099/chunks/",
            );
          } else if (name === "oversized-chunk") {
            body = Buffer.alloc(262_145, 0x61);
            headers["x-live-chunk-bytes"] = String(body.byteLength);
            headers["x-live-chunk-sha256"] = createHash("sha256").update(body).digest("hex");
          } else if (name === "truncated-chunk") {
            body = Buffer.alloc(4, 0x61);
          } else {
            url = url.replace(/\/chunks\/0$/u, "/chunks/1");
          }
          const hostile = await route.fetch({ headers, postData: body, url });
          const responseBody = await hostile.text();
          intercepted = {
            body: responseBody,
            grant,
            status: hostile.status(),
          };
          await route.fulfill({ body: responseBody, response: hostile });
        });

        await page.getByLabel("Iteration 004 file").setInputFiles({
          buffer: Buffer.alloc(8, 0x61),
          mimeType: "application/octet-stream",
          name: `${name}.bin`,
        });
        const progress = page.locator("#iteration-upload-progress");
        await expect(progress, name).toHaveAttribute("data-live-upload-state", "failed");
        await page.unroute(routePattern);
        expect(intercepted, name).not.toBeNull();
        const failure = intercepted as unknown as InterceptedUploadFailure;
        expect(failure.status, name).toBe(expectedStatus);
        expect(Buffer.byteLength(failure.body), name).toBeLessThanOrEqual(4_096);
        expect(JSON.parse(failure.body), name).toEqual({ error: expectedCode });
        expect(sentinels, name).not.toContain(failure.grant);
        sentinels.push(failure.grant);
        expect(new Set(sentinels).size, name).toBe(sentinels.length);
        await expect(progress, name).toHaveAttribute("aria-invalid", "true");
        await expect(progress, name).toHaveAttribute("aria-errormessage", "iteration-upload-error");
        await expect(page.locator("#iteration-upload-error"), name).toBeVisible();
        await expect(page.locator("#iteration-upload-error"), name).toContainText("Upload failed");
        await assertNoGrantLeak(page, sentinels);
        for (const sentinel of sentinels) {
          expect(consoleDiagnostics.join("\n"), name).not.toContain(sentinel);
          expect(pageDiagnostics.join("\n"), name).not.toContain(sentinel);
        }

        await page.getByRole("button", { name: "Retry upload" }).click();
        await expect(progress, name).toHaveAttribute("data-live-upload-state", "ready");
        await expect(page.locator("#iteration-upload-error"), name).toBeHidden();
        const nextAction = await runOrdinaryAction(page);
        expect(nextAction.revision, name).toBe(previousAction.revision + 1);
        expect(nextAction.domain_count, name).toBe(previousAction.domain_count + 1);
        previousAction = nextAction;
        await assertNoGrantLeak(page, sentinels);

        await page.getByRole("button", { name: "Remove upload" }).click();
        await resetUploadCreationWindow(page);
      }
    }

    expect((await snapshot(page)).errors).toEqual([]);
    expect((await snapshot(page)).cspViolations).toEqual([]);
  });
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} runtime observes the real scanner timeout and leaves ordinary Live usable`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    const before = await snapshot(page);

    await command(page, "armScanTimeout");
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from([
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0,
        0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
      ]),
      mimeType: "image/png",
      name: "scan-timeout.png",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "failed",
    );
    await expect(page.locator("#iteration-upload-error")).toContainText("Upload failed");
    const after = await snapshot(page);
    expect(after.host.validation_scan_calls).toBe(before.host.validation_scan_calls + 1);
    expect(after.host.active_physical_transports).toBe(0);
    expect(after.host.logical_memberships).toBe(0);
    const action = await runOrdinaryAction(page);
    expect(action.domain_count).toBeGreaterThan(0);

    await command(page, "cancelSelectedUpload");
    await resetUploadCreationWindow(page);
    expect((await snapshot(page)).errors).toEqual([]);
  });
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} runtime rejects a hostile media header through real validation`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    const before = await snapshot(page);
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      mimeType: "image/png",
      name: "truncated.png",
    });
    const progress = page.locator("#iteration-upload-progress");
    await expect(progress).toHaveAttribute("data-live-upload-state", "failed");
    await expect(page.locator("#iteration-upload-error")).toContainText("Upload failed");
    const after = await snapshot(page);
    expect(after.host.validation_scan_calls).toBe(before.host.validation_scan_calls);
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    await resetUploadCreationWindow(page);
  });
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} runtime preserves the real idempotent completion outcome`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from("duplicate-completion"),
      mimeType: "application/octet-stream",
      name: "duplicate.bin",
    });
    const progress = page.locator("#iteration-upload-progress");
    await expect(progress).toHaveAttribute("data-live-upload-state", "ready");
    const repeated = await commandResult<{ readonly revision: number; readonly state: string }>(
      page,
      "repeatSelectedCompletion",
    );
    expect(repeated.state).toBe("ready");
    await expect(progress).toHaveAttribute("data-live-completion-disposition", "existing_outcome");
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    await command(page, "cancelSelectedUpload");
    await resetUploadCreationWindow(page);
  });
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} runtime observes provider compensation and real finalizer recovery`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from("provider"),
      mimeType: "application/octet-stream",
      name: "provider.bin",
    });
    const progress = page.locator("#iteration-upload-progress");
    await expect(progress).toHaveAttribute("data-live-upload-state", "ready");

    await command(page, "armProviderCommitFailure");
    expect(await commandResult<string>(page, "finalizeSelectedUpload")).toBe(
      "upload_provider_unavailable",
    );
    await expect(progress).toHaveAttribute(
      "data-live-finalize-disposition",
      "upload_provider_unavailable",
    );
    const afterFailure = await page.request.get(
      `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
    );
    const failedInspection = (await afterFailure.json()) as {
      finalizer_commit_calls: number;
      finalizer_compensation_calls: number;
    };
    expect(failedInspection.finalizer_commit_calls).toBeGreaterThanOrEqual(1);
    expect(failedInspection.finalizer_compensation_calls).toBeGreaterThanOrEqual(1);

    expect(await commandResult<string>(page, "finalizeSelectedUpload")).toBe("finalized");
    await expect(progress).toHaveAttribute("data-live-finalize-disposition", "finalized");
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    await resetUploadCreationWindow(page);
  });
}

for (const format of ["esm", "classic"] as const) {
  for (const [caseName, disposition, recovery] of [
    ["reordered-message", "sequence_gap", "fresh_render"],
    ["replay-overflow", "invalid_envelope", "fresh_render"],
    ["revoked-authorization", "authorization_lost", "reauthorize"],
    ["fanout-pressure", "async_fanout_exceeded", "reconnect"],
  ] as const) {
    test(`${format} runtime observes real ${caseName} authority and bounded recovery`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await waitForHostQuiescent(page);
      await page.goto(
        `${REFERENCE_ORIGIN}/scenario/iteration004?features=async&format=${format}&transport=sse${caseName === "revoked-authorization" ? "&islands=2" : ""}${caseName === "reordered-message" || caseName === "replay-overflow" ? "&controlled-clock=true" : ""}`,
      );
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await expect(page.locator("[data-live-stream-state]").first()).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );

      const outcome = await page.evaluate(async (selectedCase) => {
        const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
        const callback: unknown =
          (typeof probe === "object" || typeof probe === "function") && probe !== null
            ? Reflect.get(probe, "runAsyncAdversarial")
            : null;
        if (typeof callback !== "function") throw new Error("iteration_004_async_probe_missing");
        return Reflect.apply(callback, probe, [selectedCase]) as Promise<AsyncAdversarialOutcome>;
      }, caseName);
      expect(outcome.disposition).toBe(disposition);
      expect(outcome.recovery).toBe(recovery);
      expect(outcome.retained_events).toBeLessThanOrEqual(outcome.ceiling_events);
      expect(outcome.retained_bytes).toBeLessThanOrEqual(outcome.ceiling_bytes);
      expect(outcome.accepted_sequence).toBeGreaterThanOrEqual(0);
      if (caseName === "revoked-authorization") expect(outcome.sibling_usable).toBe(true);

      if (caseName === "reordered-message" || caseName === "replay-overflow") {
        await expect(page.locator("[data-live-stream-state]").first()).toHaveAttribute(
          "data-live-stream-state",
          "degraded",
        );
        await command(page, "advanceTransportReconnect");
      }
      await expect(page.locator("[data-live-stream-state]").first()).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );
      const action = await runOrdinaryAction(page);
      expect(action.domain_count).toBeGreaterThan(0);
      const state = await snapshot(page);
      expect(state.cspViolations).toEqual([]);
      expect(state.errors).toEqual([]);
      expect(state.host.active_physical_transports).toBeLessThanOrEqual(1);
      expect(state.host.logical_memberships).toBeLessThanOrEqual(
        caseName === "revoked-authorization" ? 2 : 1,
      );
    });
  }
}

for (const format of ["esm", "classic"] as const) {
  for (const [caseName, closeReason] of [
    ["oversized-message", "frame_too_large"],
    ["truncated-message", "invalid_envelope"],
  ] as const) {
    test(`${format} runtime receives typed ${caseName} closure from the real WebSocket host`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await waitForHostQuiescent(page);
      await page.goto(
        `${REFERENCE_ORIGIN}/scenario/iteration004?features=async&format=${format}&transport=websocket`,
      );
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );
      const before = await snapshot(page);
      await page.evaluate((selectedCase) => {
        const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
        const callback: unknown =
          (typeof probe === "object" || typeof probe === "function") && probe !== null
            ? Reflect.get(probe, "sendHostileWebSocket")
            : null;
        if (typeof callback !== "function") throw new Error("iteration_004_ws_probe_missing");
        Reflect.apply(callback, probe, [selectedCase]);
      }, caseName);
      await expect
        .poll(async () => (await snapshot(page)).transportFailures)
        .toContain(`ws-close:1008:${closeReason}`);
      await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
        "data-live-stream-state",
        "current",
      );
      const after = await snapshot(page);
      expect(after.host.active_physical_transports).toBe(1);
      expect(after.host.logical_memberships).toBe(1);
      expect(after.host.physical_websocket_connections).toBe(
        before.host.physical_websocket_connections + 1,
      );
      expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    });
  }
}

for (const format of ["esm", "classic"] as const) {
  for (const [caseName, terminalState] of [
    ["cancel-finalize-cancel-wins", "canceled"],
    ["cancel-finalize-finalize-wins", "finalized"],
    ["expire-finalize-expire-wins", "expired"],
    ["expire-finalize-finalize-wins", "finalized"],
  ] as const) {
    test(`${format} runtime observes actual concurrent ${caseName}`, async ({ page }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await waitForHostQuiescent(page);
      await page.goto(
        `${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`,
      );
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await page.getByLabel("Iteration 004 file").setInputFiles({
        buffer: Buffer.from(`race:${caseName}`),
        mimeType: "application/octet-stream",
        name: `${caseName}.bin`,
      });
      const progress = page.locator("#iteration-upload-progress");
      await expect(progress).toHaveAttribute("data-live-upload-state", "ready");

      const outcome = await page.evaluate(async (selectedCase) => {
        const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
        const callback: unknown =
          (typeof probe === "object" || typeof probe === "function") && probe !== null
            ? Reflect.get(probe, "runUploadRace")
            : null;
        if (typeof callback !== "function") throw new Error("iteration_004_race_probe_missing");
        return Reflect.apply(callback, probe, [selectedCase]) as Promise<UploadRaceOutcome>;
      }, caseName);
      expect(outcome).toEqual({
        accepted_outcomes: 1,
        active_uploads: 0,
        disposition: "upload_conflict",
        terminal_state: terminalState,
      });
      await expect(progress).toHaveAttribute("data-live-race-disposition", "upload_conflict");
      await expect(progress).toHaveAttribute("data-live-upload-state", terminalState);
      expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
      await resetUploadCreationWindow(page);
    });
  }
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} runtime retirement ignores one actual late envelope and drains its owners`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(
      `${REFERENCE_ORIGIN}/scenario/iteration004?features=async&format=${format}&transport=websocket`,
    );
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
    await command(page, "holdNextEnvelope");
    await command(page, "emitNextEnvelope");
    await expect.poll(async () => (await snapshot(page)).heldEnvelopes).toBe(1);
    await command(page, "shutdown");
    await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(0);
    const retiredDom = await page.locator("[data-suprnova-live-island]").innerHTML();
    await command(page, "releaseRetiredEnvelopes");
    await expect.poll(async () => (await snapshot(page)).retiredEnvelopeAttempts).toBe(1);
    expect(await page.locator("[data-suprnova-live-island]").innerHTML()).toBe(retiredDom);
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
  });

  test(`${format} incompatible async feature closes only its dependent surface`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(
      `${REFERENCE_ORIGIN}/scenario/iteration004?features=both&format=${format}&async-artifact=incompatible`,
    );
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "disconnected",
    );
    const state = await snapshot(page);
    expect(state.host.active_physical_transports).toBe(0);
    expect(state.host.logical_memberships).toBe(0);
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from("unrelated-upload"),
      mimeType: "application/octet-stream",
      name: "unrelated.bin",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "ready",
    );
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    await command(page, "cancelSelectedUpload");
    await resetUploadCreationWindow(page);
  });

  test(`${format} upload-scope exhaustion is exact while unrelated Live remains usable`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await waitForHostQuiescent(page);
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    await command(page, "primeUploadExhaustion");
    expect((await snapshot(page)).host.active_uploads).toBe(64);
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from("ninth"),
      mimeType: "application/octet-stream",
      name: "ninth.bin",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "failed",
    );
    expect((await snapshot(page)).host.active_uploads).toBe(64);
    expect((await runOrdinaryAction(page)).domain_count).toBeGreaterThan(0);
    await command(page, "clearUploadExhaustion");
    expect((await snapshot(page)).host.active_uploads).toBe(0);
  });
}

test("every hostile Origin rejects before browser session authority or transport allocation", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  const baselineResponse = await page.request.get(
    `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
  );
  expect(baselineResponse.status()).toBe(200);
  const baseline = (await baselineResponse.json()) as {
    readonly physical_websocket_connections: number;
    readonly websocket_authentication_attempts: number;
  };
  await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=esm`);
  const cookies = await page.context().cookies(`${REFERENCE_ORIGIN}/__live/async/ws`);
  expect(cookies).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        name: "suprnova_live_reference_session",
        value: "task1-reference-session",
      }),
    ]),
  );
  const cases = [
    ["missing", []],
    ["null", ["Origin: null"]],
    ["wildcard", ["Origin: *"]],
    ["malformed", ["Origin: https://user@example.test/path"]],
    ["unapproved", ["Origin: https://cross-site.example"]],
    ["cross-site", ["Origin: http://127.0.0.1:4173"]],
    ["duplicate", [`Origin: ${REFERENCE_ORIGIN}`, `Origin: ${REFERENCE_ORIGIN}`]],
  ] as const;
  for (const [name, originHeaders] of cases) {
    const response = await rawWebSocketUpgrade([
      ...originHeaders,
      "Authorization: Bearer deliberately-invalid-session",
      "Cookie: suprnova_live_reference_session=deliberately-invalid-session",
    ]);
    expect(response.status, name).toBe(403);
    expect(Buffer.byteLength(response.body), name).toBeLessThanOrEqual(4_096);
    expect(JSON.parse(response.body), name).toEqual({ error: "websocket_origin_rejected" });
    expect(response.body, name).not.toContain(AUTHORIZATION);
    expect(response.body, name).not.toContain("deliberately-invalid-session");
  }

  const inspection = await page.request.get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`);
  expect(inspection.status()).toBe(200);
  const after = (await inspection.json()) as {
    readonly active_physical_transports: number;
    readonly logical_memberships: number;
    readonly physical_websocket_connections: number;
    readonly websocket_authentication_attempts: number;
  };
  expect(after).toMatchObject({
    active_physical_transports: 0,
    logical_memberships: 0,
  });
  expect(after.physical_websocket_connections).toBe(baseline.physical_websocket_connections);
  expect(after.websocket_authentication_attempts).toBe(baseline.websocket_authentication_attempts);
  const validOriginInvalidAuthority = await rawWebSocketUpgrade([
    `Origin: ${REFERENCE_ORIGIN}`,
    "Authorization: Bearer deliberately-invalid-session",
    "Cookie: suprnova_live_reference_session=deliberately-invalid-session",
  ]);
  expect(validOriginInvalidAuthority.status).toBe(401);
  expect(JSON.parse(validOriginInvalidAuthority.body)).toEqual({
    error: "session_authority_invalid",
  });
  const afterAuthority = await page.request.get(
    `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
  );
  expect(afterAuthority.status()).toBe(200);
  expect(
    ((await afterAuthority.json()) as { websocket_authentication_attempts: number })
      .websocket_authentication_attempts,
  ).toBe(baseline.websocket_authentication_attempts + 1);
  const manifest = await page.request.get(`${REFERENCE_ORIGIN}/suprnova-live.assets.json`);
  expect(manifest.status()).toBe(200);
  const ordinary = await page.request.post(`${REFERENCE_ORIGIN}/scenario/iteration004/action`, {
    data: {},
    headers: { Authorization: AUTHORIZATION },
  });
  expect(ordinary.status()).toBe(200);
});

test("reconnect storms retire each transport while one document transport and ordinary actions survive", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  await page.goto(scenario("&controlled-clock=true"));
  await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
    "data-live-stream-state",
    "current",
  );
  for (let reconnect = 0; reconnect < 8; reconnect += 1) {
    const before = await snapshot(page);
    await command(page, "freeze");
    await expect.poll(async () => (await snapshot(page)).host.active_physical_transports).toBe(0);
    await expect.poll(async () => (await snapshot(page)).host.logical_memberships).toBe(0);
    await command(page, "resume");
    await expect
      .poll(async () => (await snapshot(page)).authorizations.length)
      .toBe(before.authorizations.length + 1);
    await expect(page.locator("[data-live-stream-state]")).toHaveAttribute(
      "data-live-stream-state",
      "current",
    );
    const current = await snapshot(page);
    expect(current.host).toMatchObject({
      active_physical_transports: 1,
      logical_memberships: 1,
    });
  }

  const after = await snapshot(page);
  expect(after.transportFailures).toEqual([]);
  expect(after.errors).toEqual([]);
  expect(after.cspViolations).toEqual([]);
  expect(after.host.active_physical_transports).toBe(1);
  expect(after.host.logical_memberships).toBe(1);
  expect((await runOrdinaryAction(page)).revision).toBeGreaterThan(0);
});
