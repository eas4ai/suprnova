import { createHash } from "node:crypto";
import { connect } from "node:net";

import { expect, test, type APIResponse, type Page } from "@playwright/test";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";
const AUTHORIZATION = "Bearer task1-reference-session";

interface Snapshot {
  readonly authorizations: readonly unknown[];
  readonly cspViolations: readonly string[];
  readonly errors: readonly string[];
  readonly host: Readonly<{
    readonly active_physical_transports: number;
    readonly logical_memberships: number;
    readonly physical_sse_connections: number;
    readonly physical_websocket_connections: number;
  }>;
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

interface AdversarialOutcome {
  readonly case: string;
  readonly ceiling_bytes: number;
  readonly ceiling_items: number;
  readonly dependent_feature_closed: boolean;
  readonly diagnostic: string;
  readonly disposition: string;
  readonly high_water_bytes: number;
  readonly high_water_items: number;
  readonly recovery: string;
  readonly retained_bytes: number;
  readonly retained_items: number;
  readonly unrelated_scope_usable: boolean;
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
  const reset = await page.request.post(
    `${REFERENCE_ORIGIN}/__test/iteration-004/control/upload/reset-creation-window`,
  );
  expect(reset.status()).toBe(204);
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
          const final = grant.endsWith("A") ? "B" : "A";
          headers["x-live-upload-grant"] = `${grant.slice(0, -1)}${final}`;
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

    expect((await snapshot(page)).errors).toEqual([]);
    expect((await snapshot(page)).cspViolations).toEqual([]);
  });
}

test("the Rust host executes every remaining DOD 31 fault with bounded recovery", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  await page.goto(scenario());
  await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
  const cases = [
    ["hostile-media-header", "media_header_unproved", "replace"],
    ["scan-timeout", "scan_retry", "retry"],
    ["provider-partial-failure", "reconciliation_required", "reconcile"],
    ["replay-overflow", "invalid_envelope", "fresh_render"],
    ["revoked-authorization", "authorization_lost", "reauthorize"],
    ["fanout-pressure", "fanout_exceeded", "reconnect"],
    ["oversized-message", "frame_too_large", "close_transport"],
    ["truncated-message", "invalid_envelope", "close_transport"],
    ["reordered-message", "sequence_gap", "fresh_render"],
    ["duplicate-completion", "existing_outcome", "none"],
    ["cancel-finalize-cancel-wins", "upload_conflict", "terminal_canceled"],
    ["cancel-finalize-finalize-wins", "upload_conflict", "terminal_finalized"],
    ["expire-finalize-expire-wins", "upload_conflict", "terminal_expired"],
    ["expire-finalize-finalize-wins", "upload_conflict", "terminal_finalized"],
    ["late-event", "retired_delivery_ignored", "none"],
    ["retirement", "retired", "none"],
    ["unknown-feature-failure", "feature_unavailable", "none"],
    ["scoped-exhaustion", "creation_rate_exceeded", "new_scope"],
  ] as const;
  let previousAction = await runOrdinaryAction(page);

  for (const [name, disposition, recovery] of cases) {
    const response = await page.evaluate(async (caseName) => {
      const result = await fetch(`/__test/iteration-004/adversarial/${caseName}`, {
        method: "POST",
      });
      return { body: await result.text(), status: result.status };
    }, name);
    expect(response.status, name).toBe(200);
    expect(Buffer.byteLength(response.body), name).toBeLessThanOrEqual(4_096);
    const outcome = JSON.parse(response.body) as AdversarialOutcome;
    expect(outcome.case, name).toBe(name);
    expect(outcome.disposition, name).toBe(disposition);
    expect(outcome.recovery, name).toBe(recovery);
    expect(outcome.retained_items, name).toBe(0);
    expect(outcome.retained_bytes, name).toBe(0);
    expect(outcome.high_water_items, name).toBeLessThanOrEqual(outcome.ceiling_items);
    expect(outcome.high_water_bytes, name).toBeLessThanOrEqual(outcome.ceiling_bytes);
    expect(outcome.dependent_feature_closed, name).toBe(true);
    expect(outcome.unrelated_scope_usable, name).toBe(true);
    expect(outcome.diagnostic.length, name).toBeLessThanOrEqual(128);
    expect(outcome.diagnostic, name).not.toContain("secret");

    const nextAction = await runOrdinaryAction(page);
    expect(nextAction.revision, name).toBe(previousAction.revision + 1);
    expect(nextAction.domain_count, name).toBe(previousAction.domain_count + 1);
    previousAction = nextAction;
    const state = await snapshot(page);
    expect(state.host.active_physical_transports, name).toBeLessThanOrEqual(1);
    expect(state.host.logical_memberships, name).toBeLessThanOrEqual(1);
  }

  const state = await snapshot(page);
  expect(state.errors).toEqual([]);
  expect(state.cspViolations).toEqual([]);
  expect(state.host.active_physical_transports).toBe(1);
  expect(state.host.logical_memberships).toBe(1);
});

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
