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
import { AsyncDocumentQueueBudget } from "../src/async-updates/subscription.js";
import type { AuthorizedLogicalSubscription, StreamPosition } from "../src/async-updates/types.js";
import { eventLoopBarrier } from "./support/event-loop-barrier.js";

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
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
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

  subscribe(subscription: AuthorizedLogicalSubscription) {
    this.subscriptions.push(subscription.subscriptionId);
    return Object.freeze({
      descriptorBinding: subscription.descriptorBinding,
      kind: "authenticated" as const,
      stream: subscription.stream,
      subscriptionId: subscription.subscriptionId,
      transportGeneration: this.request.transportGeneration,
    });
  }

  unsubscribe(subscriptionId: string): void {
    this.unsubscribed.push(subscriptionId);
  }
}

/**
 * An SSE port whose membership acknowledgments travel separately from its
 * event stream, as they do over HTTP: the test decides when each subscribe
 * control settles and when each stream record arrives.
 */
class DeferredAcknowledgmentEventSource implements EventSourcePort {
  readonly close = vi.fn();
  readonly #pending = new Map<string, (outcome: unknown) => void>();

  constructor(readonly request: DocumentTransportConnectRequest) {}

  open(): void {
    this.request.opened();
  }

  emit(encoded: string): void {
    this.request.message(encoded);
  }

  subscribe(subscription: AuthorizedLogicalSubscription) {
    return new Promise<never>((resolve) => {
      this.#pending.set(subscription.subscriptionId, (outcome) => {
        resolve(outcome as never);
      });
    });
  }

  acknowledge(subscription: AuthorizedLogicalSubscription): void {
    this.#settle(subscription.subscriptionId, {
      descriptorBinding: subscription.descriptorBinding,
      kind: "authenticated",
      stream: subscription.stream,
      subscriptionId: subscription.subscriptionId,
      transportGeneration: this.request.transportGeneration,
    });
  }

  reject(subscription: AuthorizedLogicalSubscription): void {
    this.#settle(subscription.subscriptionId, { kind: "rejected", reason: "authorization_lost" });
  }

  unsubscribe(subscriptionId: string): void {
    this.#pending.delete(subscriptionId);
  }

  #settle(subscriptionId: string, outcome: unknown): void {
    const settle = this.#pending.get(subscriptionId);
    if (settle === undefined) throw new Error("membership_control_missing");
    this.#pending.delete(subscriptionId);
    settle(Object.freeze(outcome));
  }
}

function deferredAcknowledgmentHarness(maxQueuedEvents?: number) {
  const queueAdmission =
    maxQueuedEvents === undefined
      ? undefined
      : new AsyncDocumentQueueBudget(maxQueuedEvents, maxQueuedEvents);
  const sources: DeferredAcknowledgmentEventSource[] = [];
  const timers = new FakeTimers();
  const port = (request: DocumentTransportConnectRequest) => {
    const source = new DeferredAcknowledgmentEventSource(request);
    sources.push(source);
    return source;
  };
  const pool = new DocumentConnectionPool({
    handshakeScheduler: new OriginHandshakeScheduler(8),
    ...(queueAdmission === undefined ? {} : { queueAdmission }),
    randomness: { number: () => 0.5 },
    timers: timers.port,
    transports: { eventSource: port, webSocket: port },
  });
  return { pool, sources, timers };
}

function heartbeat(index: number, sequence: number): string {
  return canonicalize({
    payload: { kind: "heartbeat" },
    position: { epoch: "1", sequence: String(sequence) },
    protocol_version: 1,
    stream: `stream-${String(index)}`,
    subscription: `subscription-${String(index).padStart(3, "0")}`,
  });
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

function logicalSink(
  envelope: (encoded: string) => void = vi.fn(),
  state: (state: import("../src/async-updates/types.js").SubscriptionState) => void = vi.fn(),
  reauthorize: (
    prior: AuthorizedLogicalSubscription,
    signal: AbortSignal,
  ) => Promise<AuthorizedLogicalSubscription> = (prior) => Promise.resolve(prior),
) {
  return {
    envelope,
    reauthorize: async (prior: AuthorizedLogicalSubscription, signal: AbortSignal) => {
      const subscription = await reauthorize(prior, signal);
      return Object.freeze({
        commit: () => "committed" as const,
        discard: () => undefined,
        proof: "authoritative_no_tail" as const,
        subscription,
      });
    },
    state,
  };
}

function sseAcknowledgment(request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0]) {
  return Object.freeze({
    connection: request.connection,
    controlNonce: request.controlNonce,
    descriptorBinding: request.subscription.descriptorBinding,
    kind: "authenticated" as const,
    operation: request.operation,
    stream: request.subscription.stream,
    subscriptionId: request.subscription.subscriptionId,
    transportGeneration: request.transportGeneration,
  });
}

describe("multiplexed document transports", () => {
  it("shares exactly one physical connection across 100 logical subscriptions", () => {
    const { pool, sources } = harness();
    const deliveries: string[] = [];
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(
        authorized(index),
        logicalSink((encoded) => deliveries.push(encoded)),
      );
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

  it("releases a never-opening handshake so the ninth document can proceed", () => {
    const scheduler = new OriginHandshakeScheduler(8);
    const timers = new FakeTimers();
    const sources: FakeEventSource[] = [];
    const pools: DocumentConnectionPool[] = [];
    for (let index = 0; index < 9; index += 1) {
      const pool = new DocumentConnectionPool({
        handshakeScheduler: scheduler,
        handshakeTimeoutMs: 100,
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new FakeEventSource(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      });
      pools.push(pool);
      pool.subscribe(authorized(index, `scope-${String(index)}`), logicalSink());
    }

    expect(sources).toHaveLength(8);
    expect(scheduler.active("https://app.example.test")).toBe(8);
    timers.flush();
    expect(sources).toHaveLength(9);
    expect(scheduler.active("https://app.example.test")).toBe(1);
    sources[8]?.open();
    expect(scheduler.active("https://app.example.test")).toBe(0);
    sources[0]?.open();
    sources[0]?.fail();
    expect(scheduler.active("https://app.example.test")).toBe(0);
    for (const pool of pools) pool.dispose();
    expect(scheduler.active("https://app.example.test")).toBe(0);
  });

  it("routes an envelope only to its exact active subscription", () => {
    const { pool, sources } = harness();
    const left = vi.fn();
    const right = vi.fn();
    const leftHandle = pool.subscribe(authorized(1), logicalSink(left));
    pool.subscribe(authorized(2), logicalSink(right));
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
      document.pool.subscribe(authorized(index, `scope-${String(index)}`), logicalSink());
    }

    expect(documents.reduce((total, document) => total + document.sources.length, 0)).toBe(8);
    documents[0]?.sources[0]?.open();
    expect(documents.reduce((total, document) => total + document.sources.length, 0)).toBe(9);
    expect(scheduler.active("https://app.example.test")).toBe(8);
  });

  it("uses bounded full-jitter reconnect and one reconnect for all logical memberships", async () => {
    const { pool, sources, timers } = harness();
    const states = vi.fn();
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(authorized(index), logicalSink(vi.fn(), states));
    }
    sources[0]?.open();
    sources[0]?.fail();

    expect(timers.pending.size).toBe(1);
    timers.flush();
    await eventLoopBarrier();
    expect(sources).toHaveLength(2);
    expect(sources.map(({ request }) => request.transportGeneration)).toEqual([1, 2]);
    expect(states).toHaveBeenCalledWith("reconnecting");
  });

  it("closes ports and timers for persisted pagehide then reauthorizes on pageshow", async () => {
    const { pool, sources, timers } = harness();
    const late = vi.fn();
    const reauthorize = vi.fn((prior: AuthorizedLogicalSubscription) =>
      Promise.resolve({
        ...prior,
        descriptorBinding: "binding-restored",
      }),
    );
    pool.subscribe(authorized(1), logicalSink(late, vi.fn(), reauthorize));
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

    await pool.resume();
    expect(reauthorize).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(sources[1]?.subscriptions).toEqual(["subscription-001"]);
  });

  it("keeps authorization uncertainty degraded and ignores a late pre-restore port", async () => {
    const { pool, sources } = harness();
    const state = vi.fn();
    const envelope = vi.fn();
    pool.subscribe(
      authorized(1),
      logicalSink(envelope, state, () => Promise.reject(new Error("authorization_unavailable"))),
    );
    sources[0]?.open();
    pool.suspend();

    await pool.resume();
    expect(state).toHaveBeenCalledWith("degraded");
    sources[0]?.emit("late-data");
    expect(envelope).not.toHaveBeenCalled();
    expect(sources).toHaveLength(1);
  });

  it("holds a record that overtakes its SSE membership acknowledgment for that membership", async () => {
    const { pool, sources } = deferredAcknowledgmentHarness();
    const first = authorized(1);
    const second = authorized(2);
    const firstEnvelopes = vi.fn();
    const secondEnvelopes = vi.fn();
    const firstStates = vi.fn();
    const secondStates = vi.fn();
    pool.subscribe(first, logicalSink(firstEnvelopes, firstStates));
    pool.subscribe(second, logicalSink(secondEnvelopes, secondStates));
    const source = sources[0];
    if (source === undefined) throw new Error("source_missing");
    source.open();
    source.acknowledge(first);
    await eventLoopBarrier();

    // The host committed the second membership and its stream record arrived
    // before the control response that acknowledges it.
    source.emit(heartbeat(2, 1));
    expect(source.close).not.toHaveBeenCalled();
    expect(secondEnvelopes).not.toHaveBeenCalled();
    source.emit(heartbeat(1, 1));
    expect(firstEnvelopes).toHaveBeenCalledWith(heartbeat(1, 1));

    source.acknowledge(second);
    await eventLoopBarrier();
    expect(secondEnvelopes).toHaveBeenCalledWith(heartbeat(2, 1));
    source.emit(heartbeat(2, 2));
    expect(secondEnvelopes.mock.calls).toEqual([[heartbeat(2, 1)], [heartbeat(2, 2)]]);
    expect(source.close).not.toHaveBeenCalled();
    expect(firstStates).not.toHaveBeenCalledWith("degraded");
    expect(secondStates).not.toHaveBeenCalledWith("degraded");
  });

  it("fails closed once held records reach the document's queued-event limit", async () => {
    const { pool, sources } = deferredAcknowledgmentHarness(2);
    const subscription = authorized(1);
    const envelopes = vi.fn();
    pool.subscribe(subscription, logicalSink(envelopes));
    const source = sources[0];
    if (source === undefined) throw new Error("source_missing");
    source.open();
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(1, 2));
    expect(source.close).not.toHaveBeenCalled();
    source.emit(heartbeat(1, 3));
    expect(source.close).toHaveBeenCalledOnce();
    source.acknowledge(subscription);
    await eventLoopBarrier();
    expect(envelopes).not.toHaveBeenCalled();
  });

  it("still fails closed on a WebSocket record ahead of its in-order acknowledgment", () => {
    const { pool, sources } = deferredAcknowledgmentHarness();
    const subscription = Object.freeze({
      ...authorized(1),
      document: Object.freeze({ ...authorized(1).document, transport: "websocket" as const }),
    });
    const envelopes = vi.fn();
    pool.subscribe(subscription, logicalSink(envelopes));
    const source = sources[0];
    if (source === undefined) throw new Error("source_missing");
    source.open();
    expect(source.close).not.toHaveBeenCalled();
    source.emit(heartbeat(1, 1));
    expect(source.close).toHaveBeenCalledOnce();
    expect(envelopes).not.toHaveBeenCalled();
  });

  it("drops a held record when its SSE membership control is rejected", async () => {
    const { pool, sources } = deferredAcknowledgmentHarness();
    const subscription = authorized(1);
    const envelopes = vi.fn();
    const states = vi.fn();
    pool.subscribe(subscription, logicalSink(envelopes, states));
    const source = sources[0];
    if (source === undefined) throw new Error("source_missing");
    source.open();
    source.emit(heartbeat(1, 1));
    source.reject(subscription);
    await eventLoopBarrier();

    expect(envelopes).not.toHaveBeenCalled();
    expect(source.close).toHaveBeenCalledOnce();
    expect(states).toHaveBeenCalledWith("degraded");
  });

  it("keeps an authenticated replay pending until its real presentation completes", () => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    const state = vi.fn();
    const commit = vi.fn(() => "pending" as const);
    const handle = pool.subscribe(
      subscription,
      logicalSink(vi.fn(), state),
      Object.freeze({
        commit,
        discard: vi.fn(),
        proof: "complete_replay" as const,
        subscription,
      }),
    );

    sources[0]?.open();

    expect(commit).toHaveBeenCalledOnce();
    expect(state).not.toHaveBeenCalledWith("degraded");
    expect(state).not.toHaveBeenCalledWith("current");

    handle.continuityProved();
    expect(state).toHaveBeenLastCalledWith("current");
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
      transportGeneration: 1,
      ...overrides,
    };
  }

  it("uses native EventSource only for the scoped session-cookie contract", () => {
    const native = vi.fn<BrowserAsyncTransportOptions["eventSource"]>(() => ({ close: vi.fn() }));
    const fetchPort = vi.fn<typeof globalThis.fetch>();
    const membership = vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(sseAcknowledgment);
    const ports = new BrowserAsyncTransportPorts({
      eventSource: native,
      fetch: fetchPort,
      membershipTimeoutMs: 5_000,
      sseMembership: membership,
      timers: new FakeTimers().port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const request = connectRequest(Object.freeze({ kind: "session_cookie" as const }));

    const port = ports.eventSource(request);
    void port.subscribe(authorized(1));
    port.unsubscribe("subscription-001");

    expect(native).toHaveBeenCalledOnce();
    expect(native.mock.calls[0]?.[0]).toBe("https://app.example.test/__live/async/events");
    expect(native.mock.calls[0]?.[1]).toEqual({ withCredentials: true });
    expect(fetchPort).not.toHaveBeenCalled();
    expect(membership.mock.calls.map(([request]) => request.operation)).toEqual([
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
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: new FakeTimers().port,
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
    expect(init?.credentials).toBe("same-origin");
    expect(init?.redirect).toBe("error");
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

  it("delivers one SSE record far past the old 64 KiB bound, split across chunks", async () => {
    // The server encodes each envelope under LIVE_ASYNC_MAX_PAYLOAD_BYTES; the
    // record reader has no size limit of its own.
    const payload = "x".repeat(2 * 1024 * 1024);
    const record = new TextEncoder().encode(`data:${payload}\n\n`);
    const chunks = Array.from({ length: 32 }, (_, index) =>
      record.subarray(
        Math.floor((index * record.byteLength) / 32),
        Math.floor(((index + 1) * record.byteLength) / 32),
      ),
    );
    const result = await deliver(chunks, 1);
    expect(result.failure).toBeNull();
    expect(result.messages).toEqual([payload]);
  });

  /** Streams `chunks` to a bearer SSE port and resolves with what it delivered. */
  async function deliver(
    chunks: readonly Uint8Array[],
    expected: number,
  ): Promise<{ readonly messages: readonly string[]; readonly failure: string | null }> {
    const messages: string[] = [];
    let settle: ((failure: string | null) => void) | undefined;
    const settled = new Promise<string | null>((resolve) => {
      settle = resolve;
    });
    const ports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(() =>
        Promise.resolve(
          new Response(
            new ReadableStream<Uint8Array>({
              start(controller) {
                for (const chunk of chunks) controller.enqueue(chunk);
              },
            }),
            { headers: { "Content-Type": "text/event-stream" } },
          ),
        ),
      ),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: new FakeTimers().port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const port = ports.eventSource(
      connectRequest(Object.freeze({ credential: "bounded-bearer", kind: "bearer" as const }), {
        failed: (reason) => settle?.(reason),
        message: (data) => {
          messages.push(data);
          if (messages.length === expected) settle?.(null);
        },
      }),
    );
    const failure = await settled;
    port.close("document_retired");
    return { messages, failure };
  }

  /** Counts the bytes `Uint8Array` copies while `run` runs. */
  async function copiedBytes(run: () => Promise<void>): Promise<number> {
    let copied = 0;
    const slice = vi.spyOn(Uint8Array.prototype, "slice").mockImplementation(function (
      this: Uint8Array,
      ...args: Parameters<Uint8Array["slice"]>
    ) {
      const result = Uint8Array.prototype.subarray.apply(this, args).map((byte) => byte);
      copied += result.byteLength;
      return result;
    });
    const set = vi.spyOn(Uint8Array.prototype, "set");
    try {
      await run();
    } finally {
      for (const call of set.mock.calls) copied += call[0].length;
      slice.mockRestore();
      set.mockRestore();
    }
    return copied;
  }

  it("MEM-004 reads many records from one chunk without copying the tail for each", async () => {
    const records = Array.from({ length: 500 }, (_, n) => `data:{"n":${String(n)}}\n\n`).join("");
    const input = new TextEncoder().encode(records);
    let delivered: readonly string[] = [];
    const copied = await copiedBytes(async () => {
      const result = await deliver([input], 500);
      expect(result.failure).toBeNull();
      delivered = result.messages;
    });
    expect(delivered).toHaveLength(500);
    expect(delivered[499]).toBe('{"n":499}');
    expect(copied).toBeLessThanOrEqual(2 * input.byteLength);
  });

  it("MEM-004 assembles a record sent a byte at a time without copying it per byte", async () => {
    const record = new TextEncoder().encode(`data:${"y".repeat(8_192)}\n\n`);
    const chunks = Array.from(record, (byte) => Uint8Array.of(byte));
    let delivered: readonly string[] = [];
    const copied = await copiedBytes(async () => {
      const result = await deliver(chunks, 1);
      expect(result.failure).toBeNull();
      delivered = result.messages;
    });
    expect(delivered).toEqual(["y".repeat(8_192)]);
    expect(copied).toBeLessThanOrEqual(4 * record.byteLength);
  });

  it("MEM-004 reads records split across two chunks at every offset", async () => {
    const stream = new TextEncoder().encode('data:abcd\n\ndata:{"n":2}\n\ndata:xyz1234567\n\n');
    for (let split = 1; split < stream.byteLength; split += 1) {
      const result = await deliver([stream.slice(0, split), stream.slice(split)], 3);
      expect(result.failure, `split at ${String(split)}`).toBeNull();
      expect(result.messages, `split at ${String(split)}`).toEqual([
        "abcd",
        '{"n":2}',
        "xyz1234567",
      ]);
    }
  });

  it("MEM-004 reads a record fed in three uneven chunks", async () => {
    const stream = new TextEncoder().encode("data:first\n\ndata:second\n\n");
    for (let one = 1; one < stream.byteLength - 1; one += 1) {
      for (let two = one + 1; two < stream.byteLength; two += 1) {
        const chunks = [stream.slice(0, one), stream.slice(one, two), stream.slice(two)];
        const result = await deliver(chunks, 2);
        expect(result.failure, `splits at ${String(one)} and ${String(two)}`).toBeNull();
        expect(result.messages).toEqual(["first", "second"]);
      }
    }
  });

  it("MEM-004 reads a network chunk larger than one record when its records are small", async () => {
    const records = Array.from({ length: 5_000 }, (_, n) => `data:{"n":${String(n)}}\n\n`).join("");
    const input = new TextEncoder().encode(records);
    expect(input.byteLength).toBeGreaterThan(66_048);
    const result = await deliver([input], 5_000);
    expect(result.failure).toBeNull();
    expect(result.messages).toHaveLength(5_000);
  });

  it("bounds active noncooperative SSE controls and drops queued ownership on close", () => {
    const timers = new FakeTimers();
    const signals: AbortSignal[] = [];
    const failed = vi.fn();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: () => ({ close: vi.fn() }),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        signals.push(request.signal);
        return new Promise(() => undefined);
      },
      timers: timers.port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const port = ports.eventSource(
      connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed }),
    );

    for (let index = 0; index < 65; index += 1) {
      void port.subscribe(authorized(index));
    }
    expect(signals).toHaveLength(8);
    expect(failed).not.toHaveBeenCalled();

    port.close("page_suspended");
    expect(signals.every(({ aborted }) => aborted)).toBe(true);
    expect(timers.pending.size).toBe(0);
  });

  it("settles timed-out SSE controls and aborts their authority signal", () => {
    const timers = new FakeTimers();
    let membershipSignal: AbortSignal | undefined;
    const failed = vi.fn();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: () => ({ close: vi.fn() }),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        membershipSignal = request.signal;
        return new Promise(() => undefined);
      },
      timers: timers.port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const port = ports.eventSource(
      connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed }),
    );
    void port.subscribe(authorized(1));

    timers.flush();

    expect(membershipSignal?.aborted).toBe(true);
    expect(failed).toHaveBeenCalledOnce();
    expect(failed).toHaveBeenCalledWith("authorization_lost");
    expect(timers.pending.size).toBe(0);
    port.close("transport_replaced");
  });

  it.each(["rejected", "timeout"] as const)(
    "settles a %s SSE unsubscribe locally without failing a sibling membership or its source",
    async (failure) => {
      const timers = new FakeTimers();
      const failed = vi.fn();
      const message = vi.fn();
      const close = vi.fn();
      const native: {
        close(): void;
        onmessage?: (event: Readonly<{ data: string }>) => void;
      } = { close };
      const controls: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0][] = [];
      const ports = new BrowserAsyncTransportPorts({
        eventSource: () => native,
        fetch: vi.fn<typeof globalThis.fetch>(),
        membershipTimeoutMs: 5_000,
        sseMembership(request) {
          controls.push(request);
          if (request.operation === "subscribe") return sseAcknowledgment(request);
          return failure === "rejected"
            ? Promise.reject(new Error("unsubscribe_rejected"))
            : new Promise(() => undefined);
        },
        timers: timers.port,
        webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
      });
      const port = ports.eventSource(
        connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed, message }),
      );
      await expect(port.subscribe(authorized(1))).resolves.toMatchObject({ kind: "authenticated" });
      await expect(port.subscribe(authorized(2))).resolves.toMatchObject({ kind: "authenticated" });

      port.unsubscribe("subscription-001");
      await Promise.resolve();
      if (failure === "timeout") timers.flush();
      await Promise.resolve();
      native.onmessage?.({ data: "sibling-still-current" });

      expect(controls.filter(({ operation }) => operation === "unsubscribe")).toHaveLength(1);
      expect(failed).not.toHaveBeenCalled();
      expect(close).not.toHaveBeenCalled();
      expect(message).toHaveBeenCalledWith("sibling-still-current");
      expect(timers.pending.size).toBe(0);
      port.close("document_retired");
    },
  );

  it("rejects an SSE acknowledgment echoed by a different physical document connection", async () => {
    const timers = new FakeTimers();
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: ReturnType<typeof sseAcknowledgment>): void;
    }[] = [];
    const leftFailed = vi.fn();
    const rightFailed = vi.fn();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: () => ({ close: vi.fn() }),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        return new Promise((resolve) => controls.push({ request, resolve }));
      },
      timers: timers.port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const left = ports.eventSource(
      connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed: leftFailed }),
    );
    const right = ports.eventSource(
      connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed: rightFailed }),
    );
    const leftOutcome = left.subscribe(authorized(1));
    const rightOutcome = right.subscribe(authorized(1));
    const leftControl = controls[0];
    const rightControl = controls[1];
    if (leftControl === undefined || rightControl === undefined) throw new Error("missing_control");

    expect(Object.keys(leftControl.request.connection)).toEqual([]);
    expect(leftControl.request.connection).not.toBe(rightControl.request.connection);
    leftControl.resolve(sseAcknowledgment(rightControl.request));
    rightControl.resolve(sseAcknowledgment(rightControl.request));

    await expect(leftOutcome).resolves.toEqual({ kind: "rejected", reason: "authorization_lost" });
    await expect(rightOutcome).resolves.toMatchObject({ kind: "authenticated" });
    expect(leftFailed).toHaveBeenCalledWith("authorization_lost");
    expect(rightFailed).not.toHaveBeenCalled();
  });

  it("binds a WebSocket membership acknowledgment to the exact signed descriptor", async () => {
    const sent: string[] = [];
    const timers = new FakeTimers();
    const native: {
      close(): void;
      onmessage?: (event: Readonly<{ data: string }>) => void;
      send(value: string): void;
    } = {
      close: vi.fn(),
      send: (value) => sent.push(value),
    };
    const ports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: timers.port,
      webSocket: () => native,
    });
    const port = ports.webSocket({
      ...connectRequest(Object.freeze({ kind: "session_cookie" as const })),
      key: Object.freeze({ ...authorized(1).document, transport: "websocket" as const }),
    });

    const attached = port.subscribe(authorized(1));

    expect(JSON.parse(sent[0] ?? "null")).toEqual({
      control_nonce: "0000000000000001",
      descriptor_binding: "binding-1",
      kind: "subscribe",
      stream: "stream-1",
      subscription: "subscription-001",
      transport_generation: 1,
    });
    native.onmessage?.({
      data: JSON.stringify({
        control_nonce: "0000000000000001",
        descriptor_binding: "binding-1",
        kind: "membership_authenticated",
        stream: "stream-1",
        subscription: "subscription-001",
        transport_generation: 1,
      }),
    });
    timers.flush();
    await expect(attached).resolves.toEqual({
      descriptorBinding: "binding-1",
      kind: "authenticated",
      stream: "stream-1",
      subscriptionId: "subscription-001",
      transportGeneration: 1,
    });
  });

  it("rejects an unsafe browser transport generation before opening a port", () => {
    const webSocket = vi.fn<BrowserAsyncTransportOptions["webSocket"]>();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: new FakeTimers().port,
      webSocket,
    });

    expect(() =>
      ports.webSocket({
        ...connectRequest(Object.freeze({ kind: "session_cookie" as const })),
        key: Object.freeze({ ...authorized(1).document, transport: "websocket" as const }),
        transportGeneration: 0,
      }),
    ).toThrow("async_transport_generation_invalid");
    expect(webSocket).not.toHaveBeenCalled();
  });

  it("rejects foreign and duplicate WebSocket membership acknowledgments", async () => {
    const failed = vi.fn();
    const native: {
      close(): void;
      onmessage?: (event: Readonly<{ data: string }>) => void;
      send(value: string): void;
    } = { close: vi.fn(), send: vi.fn() };
    const timers = new FakeTimers();
    const ports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: timers.port,
      webSocket: () => native,
    });
    const port = ports.webSocket({
      ...connectRequest(Object.freeze({ kind: "session_cookie" as const }), { failed }),
      key: Object.freeze({ ...authorized(1).document, transport: "websocket" as const }),
    });
    const attached = port.subscribe(authorized(1));
    const acknowledgment = {
      control_nonce: "0000000000000001",
      descriptor_binding: "binding-1",
      kind: "membership_authenticated",
      stream: "stream-1",
      subscription: "subscription-001",
      transport_generation: 1,
    };

    native.onmessage?.({
      data: `{"control_nonce":"0000000000000001","descriptor_binding":"binding-1","kind":"membership_authenticated","kind":"membership_authenticated","stream":"stream-1","subscription":"subscription-001","transport_generation":1}`,
    });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");
    native.onmessage?.({
      data: JSON.stringify({ ...acknowledgment, descriptor_binding: "foreign" }),
    });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");
    native.onmessage?.({ data: JSON.stringify({ ...acknowledgment, stream: "foreign-stream" }) });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");
    native.onmessage?.({
      data: JSON.stringify({ ...acknowledgment, transport_generation: 2 }),
    });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");
    native.onmessage?.({ data: JSON.stringify(acknowledgment) });
    await expect(attached).resolves.toMatchObject({ kind: "authenticated" });
    native.onmessage?.({ data: JSON.stringify(acknowledgment) });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");

    const timedOut = port.subscribe(authorized(2));
    timers.flush();
    await expect(timedOut).resolves.toEqual({ kind: "rejected", reason: "timeout" });
    native.onmessage?.({
      data: JSON.stringify({
        ...acknowledgment,
        control_nonce: "0000000000000002",
        descriptor_binding: "binding-2",
        subscription: "subscription-002",
      }),
    });
    expect(failed).toHaveBeenLastCalledWith("protocol_invalid");
  });
});
