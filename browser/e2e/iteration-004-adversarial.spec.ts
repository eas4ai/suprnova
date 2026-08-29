import { createHash } from "node:crypto";

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

test("hostile upload authority and chunk shapes fail closed without poisoning ordinary actions", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  await page.goto(scenario());
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

test("cross-site WebSocket attempts fail before allocation and leave ordinary HTTP usable", async ({
  page,
}) => {
  await waitForHostQuiescent(page);
  const baselineResponse = await page.request.get(
    `${REFERENCE_ORIGIN}/__test/iteration-004/inspection`,
  );
  expect(baselineResponse.status()).toBe(200);
  const baseline = (await baselineResponse.json()) as {
    readonly physical_websocket_connections: number;
  };
  await page.goto("http://127.0.0.1:4173/health");
  const outcome = await page.evaluate(async () => {
    return new Promise<string>((resolve) => {
      const socket = new WebSocket("ws://127.0.0.1:4175/__live/async/ws");
      socket.addEventListener(
        "open",
        () => {
          resolve("opened");
        },
        { once: true },
      );
      socket.addEventListener(
        "error",
        () => {
          resolve("rejected");
        },
        { once: true },
      );
    });
  });
  expect(outcome).toBe("rejected");

  const inspection = await page.request.get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`);
  expect(inspection.status()).toBe(200);
  const after = (await inspection.json()) as {
    readonly active_physical_transports: number;
    readonly logical_memberships: number;
    readonly physical_websocket_connections: number;
  };
  expect(after).toMatchObject({
    active_physical_transports: 0,
    logical_memberships: 0,
  });
  expect(after.physical_websocket_connections).toBe(baseline.physical_websocket_connections);
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
