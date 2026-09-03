import { describe, expect, it } from "vitest";

import {
  BrowserAsyncAuthority,
  browserSseMembership,
  decodeAuthorizedSubscription,
} from "../src/async-updates/browser-host.js";

const SUBSCRIPTION = {
  authorization: { kind: "bearer", credential: "cred-1" },
  baseline: { epoch: "1788401008023", sequence: "0" },
  descriptor_binding: "binding-1",
  document: { authorization_scope: "scope-1", origin: "http://127.0.0.1:4178", transport: "sse" },
  events: [
    {
      cycle: { kind: "forbid_repeated_island" },
      maximumFanout: 1,
      name: "activity.posted",
      order: "per_source_sequence",
      payloadContract: "activity.posted",
      schema: "json",
      source: "stream",
      targets: ["self"],
      version: 1,
    },
    {
      cycle: { kind: "maximum_hops", maximumHops: 2 },
      maximumFanout: 4,
      name: "activity.archived",
      order: "per_source_sequence",
      payloadContract: "activity.archived",
      schema: "string",
      source: "stream",
      targets: ["self", "document"],
      version: 1,
    },
  ],
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

function fakeFetch(handler: (url: string, init: RequestInit) => Response): typeof fetch {
  const fake: typeof fetch = (input, init) =>
    Promise.resolve(handler(input instanceof Request ? input.url : input.toString(), init ?? {}));
  return fake;
}

function bodyText(init: RequestInit | undefined): string {
  return typeof init?.body === "string" ? init.body : "";
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("the default browser async host", () => {
  it("issues a subscription through the reserved route with the browser's credentials", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const authority = new BrowserAsyncAuthority({
      fetch: fakeFetch((url, init) => {
        calls.push({ url, init });
        return json(201, {
          proof: "authoritative_no_tail",
          replay: [],
          subscription: SUBSCRIPTION,
        });
      }),
      origin: "http://127.0.0.1:4178",
    });
    const result = await authority.authorize({
      identity: { component: "app.activity-feed", documentKey: "dashboard-feed", slot: "feed" },
      position: null,
      prior: null,
      signal: new AbortController().signal,
      stream: "activity",
    });
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe("http://127.0.0.1:4178/__live/v1/async/subscriptions");
    const init = calls[0]?.init ?? {};
    expect(init.method).toBe("POST");
    expect(init.credentials).toBe("same-origin");
    expect(init.redirect).toBe("error");
    expect((init.headers as Record<string, string>)["X-Suprnova-Live"]).toBe("async-v1");
    const body = JSON.parse(bodyText(init)) as Record<string, unknown>;
    expect(body["operation"]).toBe("issue");
    expect(body["protocol_version"]).toBe(1);
    expect(body["transport"]).toBe("sse");
    expect(body["stream"]).toBe("activity");
    expect(body["island"]).toEqual({
      component: "app.activity-feed",
      document_key: "dashboard-feed",
      slot: "feed",
    });
    expect(String(body["document_instance"])).toMatch(/^[A-Za-z0-9_-]{22}$/u);
    expect(result.replay).toEqual([]);
    expect(result.subscription.subscriptionId).toBe("sub-1");
    expect(result.subscription.baseline).toEqual({ epoch: 1788401008023n, sequence: 0n });
    expect(result.subscription.authorization).toEqual({ kind: "bearer", credential: "cred-1" });
    expect(result.subscription.document.transport).toBe("sse");
    expect(result.subscription.events[0]?.name).toBe("activity.posted");
    expect(result.subscription.fallbackPoll.intervalMs).toBe(30000);
    expect(result.subscription.reconnect.kind).toBe("refresh_on_reconnect");
  });

  it("renews with the prior identity and position and keeps the same document instance", async () => {
    const bodies: Record<string, unknown>[] = [];
    const authority = new BrowserAsyncAuthority({
      fetch: fakeFetch((_url, init) => {
        bodies.push(JSON.parse(bodyText(init)) as Record<string, unknown>);
        return json(200, {
          proof: "complete_replay",
          replay: ["envelope-1"],
          subscription: SUBSCRIPTION,
        });
      }),
      origin: "http://127.0.0.1:4178",
    });
    const identity = {
      component: "app.activity-feed",
      documentKey: "dashboard-feed",
      slot: "feed",
    };
    const first = await authority.authorize({
      identity,
      position: null,
      prior: null,
      signal: new AbortController().signal,
      stream: "activity",
    });
    const renewed = await authority.authorize({
      identity,
      position: { epoch: 1788401008023n, sequence: 7n },
      prior: first.subscription,
      signal: new AbortController().signal,
      stream: "activity",
    });
    expect(bodies[1]?.["operation"]).toBe("renew");
    expect(bodies[1]?.["prior"]).toEqual({
      descriptor_binding: "binding-1",
      subscription_id: "sub-1",
    });
    expect(bodies[1]?.["position"]).toEqual({ epoch: "1788401008023", sequence: "7" });
    expect(bodies[1]?.["document_instance"]).toBe(bodies[0]?.["document_instance"]);
    expect(renewed.replay).toEqual(["envelope-1"]);
  });

  it("fails closed on a rejected or malformed authority answer", async () => {
    const rejected = new BrowserAsyncAuthority({
      fetch: fakeFetch(() => json(403, { error: "x" })),
      origin: "http://127.0.0.1:4178",
    });
    const request = {
      identity: { component: "c", documentKey: "k", slot: "s" },
      position: null,
      prior: null,
      signal: new AbortController().signal,
      stream: "activity",
    };
    await expect(rejected.authorize(request)).rejects.toThrow("async_authority_rejected_403");
    const malformed = new BrowserAsyncAuthority({
      fetch: fakeFetch(() =>
        json(201, {
          replay: [],
          subscription: { ...SUBSCRIPTION, baseline: { epoch: "x", sequence: "0" } },
        }),
      ),
      origin: "http://127.0.0.1:4178",
    });
    await expect(malformed.authorize(request)).rejects.toThrow("async_authority_invalid");
    expect(() =>
      decodeAuthorizedSubscription({ ...SUBSCRIPTION, authorization: { kind: "magic" } }),
    ).toThrow("async_authority_invalid");
  });

  it("drives membership control with the bearer credential and maps the answer", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const subscription = decodeAuthorizedSubscription(SUBSCRIPTION);
    const connection = Object.freeze({}) as never;
    const base = {
      connection,
      controlNonce: "0000000000000001",
      key: {
        authorizationScope: "scope-1",
        origin: "http://127.0.0.1:4178",
        transport: "sse" as const,
      },
      signal: new AbortController().signal,
      subscription,
      transportGeneration: 1,
    };
    const acknowledged = await browserSseMembership(
      { ...base, operation: "subscribe" },
      fakeFetch((url, init) => {
        calls.push({ url, init });
        return json(200, {
          control_nonce: "0000000000000001",
          descriptor_binding: "binding-1",
          kind: "authenticated",
          operation: "subscribe",
          stream: "activity",
          subscription_id: "sub-1",
          transport_generation: 1,
        });
      }),
    );
    expect(calls[0]?.url).toBe("http://127.0.0.1:4178/__live/v1/async/memberships");
    expect((calls[0]?.init.headers as Record<string, string>)["Authorization"]).toBe(
      "SuprnovaAsync cred-1",
    );
    expect(JSON.parse(bodyText(calls[0]?.init))).toEqual({
      control_nonce: "0000000000000001",
      descriptor_binding: "binding-1",
      operation: "subscribe",
      protocol_version: 1,
      stream: "activity",
      subscription_id: "sub-1",
      transport_generation: 1,
    });
    expect(acknowledged.kind).toBe("authenticated");
    const lost = await browserSseMembership(
      { ...base, operation: "subscribe" },
      fakeFetch(() => json(403, {})),
    );
    expect(lost).toEqual({ kind: "rejected", reason: "authorization_lost" });
    const full = await browserSseMembership(
      { ...base, operation: "subscribe" },
      fakeFetch(() => json(409, {})),
    );
    expect(full).toEqual({ kind: "rejected", reason: "capacity" });
    const mismatched = await browserSseMembership(
      { ...base, operation: "subscribe" },
      fakeFetch(() =>
        json(200, {
          kind: "authenticated",
          subscription_id: "other",
          descriptor_binding: "binding-1",
          stream: "activity",
          control_nonce: "0000000000000001",
          transport_generation: 1,
        }),
      ),
    );
    expect(mismatched).toEqual({ kind: "rejected", reason: "closed" });
  });
});
