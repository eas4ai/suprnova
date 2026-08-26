import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import {
  BrowserAsyncTransportPorts,
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type AsyncTransportPorts,
  type BrowserAsyncTransportOptions,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription, StreamPosition } from "../src/async-updates/types.js";

function position(epoch: bigint, sequence: bigint): StreamPosition {
  return Object.freeze({ epoch, sequence });
}

function authorized(index: number, authorizationScope = "shared"): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: position(1n, 0n),
    descriptorBinding: `binding-${String(index)}`,
    document: Object.freeze({
      authorizationScope,
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([]),
    expiresAt: 10_000,
    heartbeatTimeoutMs: 30_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 30_000,
      minimumDelayMs: 250,
    }),
    stream: `stream-${String(index)}`,
    subscriptionId: `subscription-${String(index).padStart(3, "0")}`,
  });
}

class FakeEventSource implements EventSourcePort {
  readonly subscriptions: string[] = [];
  readonly unsubscribed: string[] = [];
  readonly close = vi.fn();

  constructor(readonly request: DocumentTransportConnectRequest) {}

  open(): void {
    this.request.opened();
  }

  emit(encoded: string): void {
    this.request.message(encoded);
  }

  fail(): void {
    this.request.failed("transport_lost");
  }

  subscribe(subscription: AuthorizedLogicalSubscription): void {
    this.subscriptions.push(subscription.subscriptionId);
  }

  unsubscribe(subscriptionId: string): void {
    this.unsubscribed.push(subscriptionId);
  }
}

class FakeTimers {
  readonly pending = new Map<number, VoidFunction>();
  #next = 0;

  readonly port = {
    clearTimeout: (handle: number) => {
      this.pending.delete(handle);
    },
    timeout: (callback: VoidFunction, milliseconds: number) => {
      void milliseconds;
      this.#next += 1;
      this.pending.set(this.#next, callback);
      return this.#next;
    },
  };

  flush(): void {
    const callbacks = [...this.pending.values()];
    this.pending.clear();
    for (const callback of callbacks) callback();
  }
}

function harness(scheduler = new OriginHandshakeScheduler(8)) {
  const sources: FakeEventSource[] = [];
  const timers = new FakeTimers();
  const transports: AsyncTransportPorts = {
    eventSource(request) {
      const source = new FakeEventSource(request);
      sources.push(source);
      return source;
    },
    webSocket() {
      throw new Error("unexpected_websocket");
    },
  };
  const pool = new DocumentConnectionPool({
    handshakeScheduler: scheduler,
    randomness: { number: () => 0.5 },
    timers: timers.port,
    transports,
  });
  return { pool, scheduler, sources, timers, transports };
}

describe("multiplexed document transports", () => {
  it("shares exactly one physical connection across 100 logical subscriptions", () => {
    const { pool, sources } = harness();
    const deliveries: string[] = [];
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(authorized(index), {
        envelope: (encoded) => deliveries.push(encoded),
        state: vi.fn(),
      });
    }

    expect(sources).toHaveLength(1);
    sources[0]?.open();
    expect(sources[0]?.subscriptions).toHaveLength(100);
    const encoded = canonicalize({
      payload: { kind: "heartbeat" },
      position: { epoch: "1", sequence: "1" },
      protocol_version: 1,
      stream: "stream-73",
      subscription: "subscription-073",
    });
    sources[0]?.emit(encoded);
    expect(deliveries).toEqual([encoded]);
  });

  it("routes an envelope only to its exact active subscription", () => {
    const { pool, sources } = harness();
    const left = vi.fn();
    const right = vi.fn();
    const leftHandle = pool.subscribe(authorized(1), { envelope: left, state: vi.fn() });
    pool.subscribe(authorized(2), { envelope: right, state: vi.fn() });
    sources[0]?.open();
    leftHandle.close();

    sources[0]?.emit(
      canonicalize({
        payload: { kind: "heartbeat" },
        position: { epoch: "1", sequence: "1" },
        protocol_version: 1,
        stream: "stream-1",
        subscription: "subscription-001",
      }),
    );
    expect(left).not.toHaveBeenCalled();
    expect(right).not.toHaveBeenCalled();
    expect(sources[0]?.unsubscribed).toEqual(["subscription-001"]);
  });

  it("limits simultaneous connection handshakes to eight per origin across documents", () => {
    const scheduler = new OriginHandshakeScheduler(8);
    const documents = Array.from({ length: 12 }, () => harness(scheduler));
    for (const [index, document] of documents.entries()) {
      document.pool.subscribe(authorized(index, `scope-${String(index)}`), {
        envelope: vi.fn(),
        state: vi.fn(),
      });
    }

    expect(documents.reduce((total, document) => total + document.sources.length, 0)).toBe(8);
    documents[0]?.sources[0]?.open();
    expect(documents.reduce((total, document) => total + document.sources.length, 0)).toBe(9);
    expect(scheduler.active("https://app.example.test")).toBe(8);
  });

  it("uses bounded full-jitter reconnect and one reconnect for all logical memberships", () => {
    const { pool, sources, timers } = harness();
    const states = vi.fn();
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(authorized(index), { envelope: vi.fn(), state: states });
    }
    sources[0]?.open();
    sources[0]?.fail();

    expect(timers.pending.size).toBe(1);
    timers.flush();
    expect(sources).toHaveLength(2);
    expect(states).toHaveBeenCalledWith("reconnecting");
  });

  it("closes ports and timers for persisted pagehide then reauthorizes on pageshow", async () => {
    const { pool, sources, timers } = harness();
    const late = vi.fn();
    pool.subscribe(authorized(1), { envelope: late, state: vi.fn() });
    sources[0]?.open();
    sources[0]?.fail();
    expect(timers.pending.size).toBe(1);

    pool.suspend();
    expect(sources[0]?.close).toHaveBeenCalledOnce();
    expect(sources[0]?.close).toHaveBeenCalledWith("transport_replaced");
    expect(timers.pending.size).toBe(0);
    sources[0]?.emit(
      canonicalize({
        payload: { kind: "heartbeat" },
        position: { epoch: "1", sequence: "1" },
        protocol_version: 1,
        stream: "stream-1",
        subscription: "subscription-001",
      }),
    );
    expect(late).not.toHaveBeenCalled();

    const reauthorize = vi.fn((prior: AuthorizedLogicalSubscription) =>
      Promise.resolve({
        ...prior,
        descriptorBinding: "binding-restored",
      }),
    );
    await pool.resume(reauthorize);
    expect(reauthorize).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(sources[1]?.subscriptions).toEqual(["subscription-001"]);
  });

  it("keeps authorization uncertainty degraded and ignores a late pre-restore port", async () => {
    const { pool, sources } = harness();
    const state = vi.fn();
    const envelope = vi.fn();
    pool.subscribe(authorized(1), { envelope, state });
    sources[0]?.open();
    pool.suspend();

    await pool.resume(() => Promise.reject(new Error("authorization_unavailable")));
    expect(state).toHaveBeenCalledWith("degraded");
    sources[0]?.emit("late-data");
    expect(envelope).not.toHaveBeenCalled();
    expect(sources).toHaveLength(1);
  });
});

describe("browser SSE authorization adapters", () => {
  function connectRequest(
    authorization: AuthorizedLogicalSubscription["authorization"],
    overrides: Partial<DocumentTransportConnectRequest> = {},
  ): DocumentTransportConnectRequest {
    return {
      authorization,
      failed: vi.fn(),
      key: authorized(1).document,
      message: vi.fn(),
      opened: vi.fn(),
      ...overrides,
    };
  }

  it("uses native EventSource only for the scoped session-cookie contract", () => {
    const native = vi.fn<BrowserAsyncTransportOptions["eventSource"]>(() => ({ close: vi.fn() }));
    const fetchPort = vi.fn<typeof globalThis.fetch>();
    const membership = vi.fn<BrowserAsyncTransportOptions["sseMembership"]>();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: native,
      fetch: fetchPort,
      sseMembership: membership,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const request = connectRequest(Object.freeze({ kind: "session_cookie" as const }));

    const port = ports.eventSource(request);
    port.subscribe(authorized(1));
    port.unsubscribe("subscription-001");

    expect(native).toHaveBeenCalledOnce();
    expect(native.mock.calls[0]?.[0]).toBe("https://app.example.test/__live/async/events");
    expect(native.mock.calls[0]?.[1]).toEqual({ withCredentials: true });
    expect(fetchPort).not.toHaveBeenCalled();
    expect(membership.mock.calls.map(([operation]) => operation)).toEqual([
      "subscribe",
      "unsubscribe",
    ]);
  });

  it("puts a bearer only in the fetch-stream authorization header and never in its URL", async () => {
    const secret = "async-bearer-secret-sentinel";
    let releaseResponse: ((response: Response) => void) | undefined;
    const fetchPort = vi.fn<typeof globalThis.fetch>(
      () =>
        new Promise<Response>((resolve) => {
          releaseResponse = resolve;
        }),
    );
    const native = vi.fn<BrowserAsyncTransportOptions["eventSource"]>();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: native,
      fetch: fetchPort,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const request = connectRequest(Object.freeze({ credential: secret, kind: "bearer" as const }));

    const port = ports.eventSource(request);
    expect(fetchPort).toHaveBeenCalledOnce();
    const [input, init] = fetchPort.mock.calls[0] ?? [];
    const requestUrl =
      typeof input === "string" ? input : input instanceof URL ? input.href : (input?.url ?? "");
    expect(requestUrl).toBe("https://app.example.test/__live/async/events");
    expect(requestUrl).not.toContain(secret);
    expect(new Headers(init?.headers).get("Authorization")).toBe(`SuprnovaAsync ${secret}`);
    expect(native).not.toHaveBeenCalled();

    releaseResponse?.(
      new Response(
        new ReadableStream<Uint8Array>({
          start(controller) {
            void controller;
          },
        }),
        {
          headers: { "Content-Type": "text/event-stream" },
        },
      ),
    );
    port.close("document_retired");
    await Promise.resolve();
  });

  it("fails a bearer stream closed when one SSE record exceeds the envelope bound", async () => {
    let signalFailure: ((reason: string) => void) | undefined;
    const failed = new Promise<string>((resolve) => {
      signalFailure = resolve;
    });
    const oversized = new TextEncoder().encode(`data:${"x".repeat(65_537)}\n\n`);
    const ports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(() =>
        Promise.resolve(
          new Response(
            new ReadableStream<Uint8Array>({
              start(controller) {
                controller.enqueue(oversized);
                controller.close();
              },
            }),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
        ),
      ),
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });

    ports.eventSource(
      connectRequest(Object.freeze({ credential: "bounded-bearer", kind: "bearer" as const }), {
        failed: (reason) => signalFailure?.(reason),
      }),
    );

    await expect(failed).resolves.toBe("protocol_invalid");
  });
});
