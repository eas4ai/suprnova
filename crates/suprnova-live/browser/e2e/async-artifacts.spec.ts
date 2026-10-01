import { readFile } from "node:fs/promises";

import { expect, test, type Page } from "@playwright/test";

type ArtifactKind = "classic" | "esm";

async function installAsyncHarness(page: Page, delayInitialAuthority = false): Promise<void> {
  await page.addInitScript((delayInitial) => {
    interface PendingMembership {
      readonly request: Readonly<{ transportGeneration: number }>;
      readonly subscription: Readonly<{
        descriptorBinding: string;
        stream: string;
        subscriptionId: string;
      }>;
      resolve(value: unknown): void;
    }
    interface PhysicalConnection {
      closed: boolean;
      readonly request: {
        message(encoded: string): void;
        opened(): void;
        readonly transportGeneration: number;
      };
    }
    const pending: PendingMembership[] = [];
    const connections: PhysicalConnection[] = [];
    const authorizations: { position: string | null; prior: string | null }[] = [];
    const initialAuthority: ((value: unknown) => void)[] = [];
    const harness = {
      ack(index: number): void {
        const membership = pending[index];
        if (membership === undefined) throw new Error("async_fixture_membership_missing");
        membership.resolve(
          Object.freeze({
            descriptorBinding: membership.subscription.descriptorBinding,
            kind: "authenticated",
            stream: membership.subscription.stream,
            subscriptionId: membership.subscription.subscriptionId,
            transportGeneration: membership.request.transportGeneration,
          }),
        );
      },
      authorizations,
      connections,
      emit(connectionIndex: number, sequence: number, open: boolean): void {
        const connection = connections[connectionIndex];
        if (connection === undefined) throw new Error("async_fixture_connection_missing");
        connection.request.message(
          JSON.stringify({
            payload: { kind: "presentation_signal", name: "open", scope: "primary", value: open },
            position: { epoch: "1", sequence: String(sequence) },
            protocol_version: 1,
            stream: "orders",
            subscription: "subscription-001",
          }),
        );
      },
      pending,
      resolveInitial(): void {
        const resolve = initialAuthority.shift();
        if (resolve === undefined) throw new Error("async_fixture_initial_authority_missing");
        resolve(
          Object.freeze({
            replay: Object.freeze([]),
            subscription: authorization(1, 0n),
          }),
        );
      },
    };
    function authorization(call: number, sequence: bigint) {
      return Object.freeze({
        authorization: Object.freeze({ kind: "session_cookie" }),
        baseline: Object.freeze({ epoch: 1n, sequence }),
        descriptorBinding: `binding-${String(call)}`,
        document: Object.freeze({
          authorizationScope: "artifact-document-scope",
          origin: window.location.origin,
          transport: "sse",
        }),
        events: Object.freeze([]),
        expiresAt: 20_000,
        fallbackPoll: Object.freeze({
          initial: "wait",
          intervalMs: 30_000,
          jitterRatio: 0.2,
          visibility: "visible",
        }),
        heartbeatTimeoutMs: 5_000,
        presentationSignals: Object.freeze([
          Object.freeze({ name: "open", schema: "boolean", scope: "primary" }),
        ]),
        reconnect: Object.freeze({
          kind: "resume_or_refresh",
          maximumAttempts: 2,
          maximumDelayMs: 400,
          minimumDelayMs: 100,
        }),
        stream: "orders",
        subscriptionId: "subscription-001",
      });
    }
    const options = Object.freeze({
      authority: Object.freeze({
        authorize(request: {
          position: Readonly<{ epoch: bigint; sequence: bigint }> | null;
          prior: Readonly<{ subscriptionId: string }> | null;
        }) {
          const sequence = request.position?.sequence ?? 0n;
          authorizations.push({
            position: request.position === null ? null : String(sequence),
            prior: request.prior?.subscriptionId ?? null,
          });
          const result = Object.freeze({
            replay: Object.freeze([]),
            subscription: authorization(authorizations.length, sequence),
          });
          return delayInitial && authorizations.length === 1
            ? new Promise((resolve) => initialAuthority.push(resolve))
            : result;
        },
      }),
      clock: Object.freeze({ now: () => 100 }),
      randomness: Object.freeze({ number: () => 0.5 }),
      timers: Object.freeze({
        clearTimeout: (handle: number) => {
          window.clearTimeout(handle);
        },
        timeout: (callback: () => void, milliseconds: number) => {
          // suprnova-correctness-delay-allow: product-timer -- injected runtime timer exercises observable reconnect policy rather than test synchronization
          return window.setTimeout(callback, milliseconds);
        },
      }),
      transports: Object.freeze({
        eventSource(request: PhysicalConnection["request"]) {
          const connection: PhysicalConnection & {
            close(): void;
            subscribe(subscription: PendingMembership["subscription"]): Promise<unknown>;
            unsubscribe(): void;
          } = {
            close() {
              connection.closed = true;
            },
            closed: false,
            request,
            subscribe(subscription) {
              return new Promise((resolve) => pending.push({ request, resolve, subscription }));
            },
            unsubscribe() {
              // The fixture has one retained logical membership.
            },
          };
          connections.push(connection);
          queueMicrotask(() => {
            if (!connection.closed) request.opened();
          });
          return connection;
        },
        webSocket() {
          throw new Error("async_fixture_unexpected_websocket");
        },
      }),
    });
    Reflect.set(window, "__suprnovaAsyncHarness", harness);
    Reflect.set(window, "__suprnovaAsyncOptions", options);
  }, delayInitialAuthority);
}

async function installArtifactRoutes(page: Page, kind: ArtifactKind): Promise<void> {
  const scenario = kind === "esm" ? "cspNonce" : "cspClassicNonce";
  await page.route(`**/scenario/${scenario}?async-artifact=${kind}`, async (route) => {
    const original = await route.fetch();
    let html = await original.text();
    html = html.replace(
      "data-suprnova-live-island",
      'data-suprnova-live-island live:stream="orders" live:signal="open:false"',
    );
    html = html.replace(
      "<p>Server-rendered search results</p>",
      '<p>Server-rendered search results</p><div id="async-panel" hidden aria-hidden="true" inert live:show="open">Async artifact applied</div>',
    );
    const scripts =
      kind === "esm"
        ? '<script type="module" src="/test-async/esm-boot.js"></script>'
        : '<script src="/test-async/suprnova-live.async.classic.js"></script><script src="/test-async/classic-config.js"></script><script src="/assets/suprnova-live.classic.js"></script><script src="/test-boot/classic.js"></script>';
    html = html.replace(/<\/main>[\s\S]*<\/body>/u, `</main>${scripts}</body>`);
    await route.fulfill({
      body: html,
      headers: {
        ...original.headers(),
        "content-security-policy": "default-src 'none'; script-src 'self'; connect-src 'self'",
        "content-type": "text/html; charset=utf-8",
      },
      status: 200,
    });
  });
  await page.route("**/test-async/suprnova-live.async.esm.js", async (route) => {
    await route.fulfill({
      body: await readFile(new URL("../dist/suprnova-live.async.esm.js", import.meta.url)),
      contentType: "text/javascript; charset=utf-8",
    });
  });
  await page.route("**/test-async/suprnova-live.async.classic.js", async (route) => {
    await route.fulfill({
      body: await readFile(new URL("../dist/suprnova-live.async.classic.js", import.meta.url)),
      contentType: "text/javascript; charset=utf-8",
    });
  });
  await page.route("**/test-async/esm-boot.js", async (route) => {
    await route.fulfill({
      body: `import { configureAsync } from "/test-async/suprnova-live.async.esm.js";
configureAsync(globalThis.__suprnovaAsyncOptions);
import { boot } from "/assets/suprnova-live.esm.js";
boot();`,
      contentType: "text/javascript; charset=utf-8",
    });
  });
  await page.route("**/test-async/classic-config.js", async (route) => {
    await route.fulfill({
      body: `globalThis[Symbol.for("suprnova.live.features.v1")].configureAsync(globalThis.__suprnovaAsyncOptions);`,
      contentType: "text/javascript; charset=utf-8",
    });
  });
}

async function harnessCounts(page: Page): Promise<{
  authorizations: number;
  connections: number;
  pending: number;
}> {
  return page.evaluate(() => {
    const value = Reflect.get(window, "__suprnovaAsyncHarness") as {
      authorizations: unknown[];
      connections: unknown[];
      pending: unknown[];
    };
    return {
      authorizations: value.authorizations.length,
      connections: value.connections.length,
      pending: value.pending.length,
    };
  });
}

async function invokeHarness(
  page: Page,
  method: "ack" | "emit" | "resolveInitial",
  ...args: unknown[]
) {
  await page.evaluate(
    ({ args: values, method: name }) => {
      const harness = Reflect.get(window, "__suprnovaAsyncHarness") as Record<string, unknown>;
      const operation = Reflect.get(harness, name);
      if (typeof operation !== "function") throw new Error("async_fixture_operation_missing");
      Reflect.apply(operation, harness, values);
    },
    { args, method },
  );
}

async function persistedTransition(page: Page, type: "pagehide" | "pageshow"): Promise<void> {
  await page.evaluate((eventType) => {
    const event = new Event(eventType);
    Object.defineProperty(event, "persisted", { value: true });
    window.dispatchEvent(event);
  }, type);
}

for (const kind of ["esm", "classic"] as const) {
  test(`production async ${kind} artifact restarts authority pending before transport on bfcache under CSP`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await installAsyncHarness(page, true);
    await installArtifactRoutes(page, kind);
    const scenario = kind === "esm" ? "cspNonce" : "cspClassicNonce";
    await page.goto(`/scenario/${scenario}?async-artifact=${kind}`);

    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 1,
        connections: 0,
        pending: 0,
      });
    await persistedTransition(page, "pagehide");
    await persistedTransition(page, "pageshow");
    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 2,
        connections: 1,
        pending: 1,
      });
    await expect
      .poll(() =>
        page.evaluate(() => {
          const harness = Reflect.get(window, "__suprnovaAsyncHarness") as {
            authorizations: { position: string | null; prior: string | null }[];
          };
          return harness.authorizations;
        }),
      )
      .toEqual([
        { position: null, prior: null },
        { position: null, prior: null },
      ]);
    await invokeHarness(page, "resolveInitial");
    await expect(page.locator("#async-panel")).toBeHidden();
    await invokeHarness(page, "ack", 0);
    await invokeHarness(page, "emit", 0, 1, true);
    await expect(page.locator("#async-panel")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test(`production async ${kind} artifact reacquires fresh initial authority after pre-ACK bfcache under CSP`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await installAsyncHarness(page);
    await installArtifactRoutes(page, kind);
    const scenario = kind === "esm" ? "cspNonce" : "cspClassicNonce";
    await page.goto(`/scenario/${scenario}?async-artifact=${kind}`);

    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 1,
        connections: 1,
        pending: 1,
      });
    await expect(page.locator("#async-panel")).toBeHidden();

    await persistedTransition(page, "pagehide");
    await persistedTransition(page, "pageshow");
    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 2,
        connections: 2,
        pending: 2,
      });
    await expect
      .poll(() =>
        page.evaluate(() => {
          const harness = Reflect.get(window, "__suprnovaAsyncHarness") as {
            authorizations: { position: string | null; prior: string | null }[];
          };
          return harness.authorizations;
        }),
      )
      .toEqual([
        { position: null, prior: null },
        { position: null, prior: null },
      ]);

    await invokeHarness(page, "ack", 0);
    await invokeHarness(page, "emit", 0, 1, true);
    await expect(page.locator("#async-panel")).toBeHidden();
    await invokeHarness(page, "ack", 1);
    await invokeHarness(page, "emit", 1, 1, true);
    await expect(page.locator("#async-panel")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test(`production async ${kind} artifact authenticates exact membership and restores lifecycle under CSP`, async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await installAsyncHarness(page);
    await installArtifactRoutes(page, kind);
    const scenario = kind === "esm" ? "cspNonce" : "cspClassicNonce";
    await page.goto(`/scenario/${scenario}?async-artifact=${kind}`);

    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 1,
        connections: 1,
        pending: 1,
      });
    await expect(page.locator("#async-panel")).toBeHidden();
    await invokeHarness(page, "ack", 0);
    await invokeHarness(page, "emit", 0, 1, true);
    await expect(page.locator("#async-panel")).toBeVisible();

    await persistedTransition(page, "pagehide");
    await persistedTransition(page, "pageshow");
    await expect
      .poll(() => harnessCounts(page))
      .toEqual({
        authorizations: 2,
        connections: 2,
        pending: 2,
      });
    await invokeHarness(page, "emit", 0, 2, false);
    await expect(page.locator("#async-panel")).toBeVisible();
    await invokeHarness(page, "ack", 1);
    await invokeHarness(page, "emit", 1, 2, false);
    await expect(page.locator("#async-panel")).toBeHidden();
    expect(errors).toEqual([]);
  });
}
