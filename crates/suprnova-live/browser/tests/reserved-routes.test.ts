import { afterEach, describe, expect, it, vi } from "vitest";

import {
  BrowserAsyncAuthority,
  browserSseMembership,
  decodeAuthorizedSubscription,
} from "../src/async-updates/browser-host.js";
import {
  BrowserAsyncTransportPorts,
  type BrowserAsyncTransportOptions,
  type DocumentTransportConnectRequest,
} from "../src/async-updates/connections.js";
import { RESERVED_ROUTES_CONFIG_ELEMENT_ID, reservedRoutePath } from "../src/reserved-routes.js";
import { CONFIG_ELEMENT_ID } from "../src/runtime/config.js";
import { FetchUploadTransport } from "../src/uploads/feature.js";

// PFX-006: behind a reverse proxy that serves the application under a path
// prefix, the server writes the action endpoint under the prefix, and every
// other reserved Live route follows it there.

const PREFIXED_CONFIG = JSON.stringify({ endpoint: "/billing/__live/action" });

/** The two members of `Document` the route reader uses. */
function page(configText: string | null, baseURI = "https://app.test/billing/catalog") {
  return {
    baseURI,
    getElementById(id: string) {
      return id === "suprnova-live-config" && configText !== null
        ? { textContent: configText }
        : null;
    },
  } as unknown as Document;
}

/** Install `fake` as the page's document for one test. */
function usePage(fake: Document): void {
  vi.stubGlobal("document", fake);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

const SUBSCRIPTION = {
  authorization: { kind: "bearer", credential: "cred-1" },
  baseline: { epoch: "1788401008023", sequence: "0" },
  descriptor_binding: "binding-1",
  document: { authorization_scope: "scope-1", origin: "https://app.test", transport: "sse" },
  events: [],
  expires_at: 1788401068023,
  fallback_poll: { initial: "wait", interval_ms: 30000, jitter_ratio: 0.2, visibility: "visible" },
  heartbeat_timeout_ms: 15000,
  presentation_signals: [],
  reconnect: {
    kind: "refresh_on_reconnect",
    maximum_attempts: 8,
    maximum_delay_ms: 30000,
    minimum_delay_ms: 500,
  },
  stream: "activity",
  subscription_id: "sub-1",
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function urlOf(input: RequestInfo | URL | undefined): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.href;
  return input?.url ?? "";
}

describe("PFX-006 reserved Live routes follow the configured endpoint", () => {
  it("PFX-006 reads the configuration element the core runtime reads", () => {
    expect(RESERVED_ROUTES_CONFIG_ELEMENT_ID).toBe(CONFIG_ELEMENT_ID);
  });

  it("PFX-006 puts every reserved route under the endpoint's root", () => {
    const fake = page(PREFIXED_CONFIG);
    expect(reservedRoutePath("upload", fake)).toBe("/billing/__live/upload");
    expect(reservedRoutePath("async/subscriptions", fake)).toBe(
      "/billing/__live/async/subscriptions",
    );
    expect(reservedRoutePath("async/memberships", fake)).toBe("/billing/__live/async/memberships");
    expect(reservedRoutePath("async/events", fake)).toBe("/billing/__live/async/events");
    expect(reservedRoutePath("async/socket", fake)).toBe("/billing/__live/async/socket");
  });

  it("PFX-006 keeps the routes at the host root without a prefix", () => {
    expect(reservedRoutePath("upload", page(JSON.stringify({ endpoint: "/__live/action" })))).toBe(
      "/__live/upload",
    );
    expect(reservedRoutePath("upload", page(null))).toBe("/__live/upload");
    expect(reservedRoutePath("upload", page("not json"))).toBe("/__live/upload");
    expect(reservedRoutePath("upload", page(JSON.stringify({ endpoint: "/live?mode=x" })))).toBe(
      "/__live/upload",
    );
    expect(reservedRoutePath("upload")).toBe("/__live/upload");
  });

  it("PFX-006 sends uploads to the upload endpoint under the root", async () => {
    usePage(page(PREFIXED_CONFIG));
    const calls: (RequestInfo | URL)[] = [];
    const transport = new FetchUploadTransport((input) => {
      calls.push(input);
      return Promise.resolve(json(200, { revision: "2", state: "transferring" }));
    });
    await transport.send({
      grant: "transfer-grant",
      handle: "018f47c1-2af0-7cc4-a001-000000000001",
      operation: "status",
      signal: new AbortController().signal,
    });
    expect(calls.map(urlOf)).toEqual(["/billing/__live/upload"]);
  });

  it("PFX-006 issues subscriptions and drives memberships under the root", async () => {
    usePage(page(PREFIXED_CONFIG));
    const urls: string[] = [];
    const fetchPort: typeof fetch = (input) => {
      urls.push(urlOf(input));
      return Promise.resolve(
        json(201, { proof: "authoritative_no_tail", replay: [], subscription: SUBSCRIPTION }),
      );
    };
    const authority = new BrowserAsyncAuthority({ fetch: fetchPort, origin: "https://app.test" });
    await authority.authorize({
      identity: { component: "app.activity-feed", documentKey: "feed", slot: "feed" },
      position: null,
      prior: null,
      signal: new AbortController().signal,
      stream: "activity",
    });
    expect(urls).toEqual(["https://app.test/billing/__live/async/subscriptions"]);

    const membershipUrls: string[] = [];
    await browserSseMembership(
      {
        connection: Object.freeze({}) as never,
        controlNonce: "0000000000000001",
        key: { authorizationScope: "scope-1", origin: "https://app.test", transport: "sse" },
        operation: "subscribe",
        signal: new AbortController().signal,
        subscription: decodeAuthorizedSubscription(SUBSCRIPTION),
        transportGeneration: 1,
      },
      (input) => {
        membershipUrls.push(urlOf(input));
        return Promise.resolve(json(403, {}));
      },
    );
    expect(membershipUrls).toEqual(["https://app.test/billing/__live/async/memberships"]);
  });

  it("PFX-006 opens the event stream and the socket under the root", () => {
    usePage(page(PREFIXED_CONFIG));
    const eventSource = vi.fn<BrowserAsyncTransportOptions["eventSource"]>(() => ({
      close: vi.fn(),
    }));
    const webSocket = vi.fn<BrowserAsyncTransportOptions["webSocket"]>(() => ({
      close: vi.fn(),
      send: vi.fn(),
    }));
    const ports = new BrowserAsyncTransportPorts({
      eventSource,
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: { clearTimeout: vi.fn(), timeout: vi.fn(() => 1) },
      webSocket,
    });
    const request = (transport: "sse" | "websocket"): DocumentTransportConnectRequest => ({
      authorization: Object.freeze({ kind: "session_cookie" as const }),
      failed: vi.fn(),
      key: { authorizationScope: "scope-1", origin: "https://app.test", transport },
      message: vi.fn(),
      opened: vi.fn(),
      transportGeneration: 1,
    });

    ports.eventSource(request("sse"));
    expect(eventSource.mock.calls[0]?.[0]).toBe("https://app.test/billing/__live/async/events");

    ports.webSocket(request("websocket"));
    expect(webSocket.mock.calls[0]?.[0]).toBe("wss://app.test/billing/__live/async/socket");
  });
});
