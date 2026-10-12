import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import {
  BrowserAsyncTransportPorts,
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type BrowserAsyncTransportOptions,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
  type LogicalSubscriptionHandle,
} from "../src/async-updates/connections.js";
import { AsyncDocumentQueueBudget } from "../src/async-updates/subscription.js";
import type {
  AuthorizedLogicalSubscription,
  SubscriptionState,
} from "../src/async-updates/types.js";
import type { LiveLimitBreach } from "../src/limits.js";
import { eventLoopBarrier } from "./support/event-loop-barrier.js";

// An SSE membership control is an HTTP request beside the event stream, so a
// record the host sends after committing a membership can reach the browser
// before the control's answer. These tests pin how the document connection
// pool holds such records: inert, against the document's queue budget, and
// applied in arrival order only after the exact acknowledgment.

function authorized(
  index: number,
  overrides: Partial<AuthorizedLogicalSubscription> = {},
): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: `binding-${String(index)}`,
    document: Object.freeze({
      authorizationScope: "shared",
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
    ...overrides,
  });
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

function only<T>(value: T | undefined): T {
  if (value === undefined) throw new Error("value_missing");
  return value;
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

/** An SSE port whose membership answers the test settles by hand. */
class DeferredSource implements EventSourcePort {
  readonly close = vi.fn();
  readonly unsubscribe = vi.fn();
  readonly #pending = new Map<string, (outcome: unknown) => void>();

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
    return new Promise<never>((resolve) => {
      this.#pending.set(subscription.subscriptionId, (outcome) => {
        resolve(outcome as never);
      });
    });
  }

  acknowledge(subscription: AuthorizedLogicalSubscription, overrides: object = {}): void {
    const settle = this.#pending.get(subscription.subscriptionId);
    if (settle === undefined) throw new Error("membership_control_missing");
    this.#pending.delete(subscription.subscriptionId);
    settle(
      Object.freeze({
        descriptorBinding: subscription.descriptorBinding,
        kind: "authenticated",
        stream: subscription.stream,
        subscriptionId: subscription.subscriptionId,
        transportGeneration: this.request.transportGeneration,
        ...overrides,
      }),
    );
  }
}

function harness(queueAdmission = new AsyncDocumentQueueBudget()) {
  const sources: DeferredSource[] = [];
  const timers = new FakeTimers();
  const port = (request: DocumentTransportConnectRequest) => {
    const source = new DeferredSource(request);
    sources.push(source);
    return source;
  };
  const pool = new DocumentConnectionPool({
    handshakeScheduler: new OriginHandshakeScheduler(8),
    queueAdmission,
    randomness: { number: () => 0.5 },
    timers: timers.port,
    transports: { eventSource: port, webSocket: port },
  });
  return { pool, queueAdmission, sources, timers };
}

function boundedBudget(maxEvents: number) {
  const breaches: LiveLimitBreach[] = [];
  const budget = new AsyncDocumentQueueBudget(maxEvents, maxEvents, (breach) => {
    breaches.push(breach);
  });
  return { breaches, budget };
}

function sink(
  envelope: (encoded: string) => void = vi.fn(),
  state: (state: SubscriptionState) => void = vi.fn(),
  successor?: AuthorizedLogicalSubscription,
) {
  return {
    envelope,
    reauthorize: (prior: AuthorizedLogicalSubscription | null) =>
      Promise.resolve(
        Object.freeze({
          commit: () => "committed" as const,
          discard: () => undefined,
          proof: "authoritative_no_tail" as const,
          subscription: successor ?? prior ?? authorized(1),
        }),
      ),
    state,
  };
}

describe("SSE records that overtake their membership acknowledgment", () => {
  it("applies every held record in arrival order, ahead of later records", async () => {
    const { pool, sources } = harness();
    const first = authorized(1);
    const second = authorized(2);
    const firstEnvelopes = vi.fn();
    const secondEnvelopes = vi.fn();
    pool.subscribe(first, sink(firstEnvelopes));
    pool.subscribe(second, sink(secondEnvelopes));
    const source = only(sources[0]);
    source.open();
    source.acknowledge(first);
    await eventLoopBarrier();

    source.emit(heartbeat(2, 1));
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(2, 2));
    source.emit(heartbeat(2, 3));
    source.acknowledge(second);
    // A record that arrives while the acknowledgment is still settling is
    // held behind the earlier ones.
    source.emit(heartbeat(2, 4));
    await eventLoopBarrier();
    source.emit(heartbeat(2, 5));

    expect(secondEnvelopes.mock.calls).toEqual([
      [heartbeat(2, 1)],
      [heartbeat(2, 2)],
      [heartbeat(2, 3)],
      [heartbeat(2, 4)],
      [heartbeat(2, 5)],
    ]);
    expect(firstEnvelopes.mock.calls).toEqual([[heartbeat(1, 1)]]);
    expect(source.close).not.toHaveBeenCalled();
  });

  it("charges held records to the document's shared queue budget and releases them", async () => {
    const { budget } = boundedBudget(4);
    const { pool, sources } = harness(budget);
    // Two envelopes the document's islands already hold leave room for two.
    expect(budget.reserve(2, 10)).toBe(true);
    const first = authorized(1);
    const second = authorized(2);
    pool.subscribe(first, sink());
    pool.subscribe(second, sink());
    const source = only(sources[0]);
    source.open();
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(2, 1));
    expect(budget.current().queuedEvents).toBe(4);
    expect(source.close).not.toHaveBeenCalled();

    source.acknowledge(first);
    source.acknowledge(second);
    await eventLoopBarrier();
    expect(budget.current()).toEqual({ queuedBytes: 10, queuedEvents: 2 });
  });

  it("counts every membership's held records against one document budget", () => {
    const { breaches, budget } = boundedBudget(3);
    const { pool, sources } = harness(budget);
    pool.subscribe(authorized(1), sink());
    pool.subscribe(authorized(2), sink());
    const source = only(sources[0]);
    source.open();
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(1, 2));
    source.emit(heartbeat(2, 1));
    expect(source.close).not.toHaveBeenCalled();
    source.emit(heartbeat(2, 2));
    expect(source.close).toHaveBeenCalledOnce();
    expect(breaches).toHaveLength(1);
  });

  it("reports a full queue as its configured limit and reconnects the transport", () => {
    const { breaches, budget } = boundedBudget(1);
    const { pool, sources, timers } = harness(budget);
    const states = vi.fn();
    pool.subscribe(authorized(1), sink(vi.fn(), states));
    const source = only(sources[0]);
    source.open();
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(1, 2));

    expect(breaches).toEqual([
      expect.objectContaining({
        configured: 1,
        key: "LIVE_ASYNC_MAX_QUEUED_EVENTS",
        limit: "asyncMaxQueuedEvents",
        measured: 2,
      }),
    ]);
    // A lost transport, not lost authority: the membership reconnects.
    expect(source.close).toHaveBeenCalledExactlyOnceWith("transport_replaced");
    expect(states).toHaveBeenLastCalledWith("reconnecting");
    expect(states).not.toHaveBeenCalledWith("degraded");
    expect(timers.pending.size).toBe(1);
    expect(budget.current().queuedEvents).toBe(0);
  });

  it("drops held records on transport loss and ignores the old generation's late answer", async () => {
    const budget = new AsyncDocumentQueueBudget();
    const { pool, sources, timers } = harness(budget);
    const subscription = authorized(1);
    const envelopes = vi.fn();
    pool.subscribe(subscription, sink(envelopes));
    only(sources[0]).open();
    only(sources[0]).emit(heartbeat(1, 1));
    only(sources[0]).fail();
    expect(budget.current().queuedEvents).toBe(0);
    only(sources[0]).acknowledge(subscription);
    await eventLoopBarrier();
    timers.flush();
    await eventLoopBarrier();
    await eventLoopBarrier();

    expect(sources).toHaveLength(2);
    only(sources[1]).open();
    only(sources[1]).emit(heartbeat(1, 9));
    only(sources[1]).acknowledge(subscription);
    await eventLoopBarrier();
    expect(envelopes.mock.calls).toEqual([[heartbeat(1, 9)]]);
    expect(only(sources[1]).close).not.toHaveBeenCalled();
  });

  it("releases a removed membership's held records from the document budget", () => {
    const { budget } = boundedBudget(2);
    const { pool, sources } = harness(budget);
    const handle = pool.subscribe(authorized(1), sink());
    pool.subscribe(authorized(2), sink());
    const source = only(sources[0]);
    source.open();
    source.emit(heartbeat(1, 1));
    source.emit(heartbeat(1, 2));
    handle.close();
    expect(budget.current().queuedEvents).toBe(0);
    source.emit(heartbeat(2, 1));
    source.emit(heartbeat(2, 2));
    expect(source.close).not.toHaveBeenCalled();
  });

  it.each([
    ["descriptor binding", { descriptorBinding: "binding-other" }],
    ["transport generation", { transportGeneration: 99 }],
    ["stream", { stream: "stream-other" }],
  ])("fails closed and drops held records on an answer with the wrong %s", async (_, overrides) => {
    const budget = new AsyncDocumentQueueBudget();
    const { pool, sources } = harness(budget);
    const subscription = authorized(1);
    const envelopes = vi.fn();
    const states = vi.fn();
    pool.subscribe(subscription, sink(envelopes, states));
    only(sources[0]).open();
    only(sources[0]).emit(heartbeat(1, 1));
    only(sources[0]).acknowledge(subscription, overrides);
    await eventLoopBarrier();
    expect(envelopes).not.toHaveBeenCalled();
    expect(only(sources[0]).close).toHaveBeenCalledOnce();
    expect(states).toHaveBeenLastCalledWith("degraded");
    expect(budget.current().queuedEvents).toBe(0);
  });

  it.each([
    [
      "closes the membership",
      (handle: LogicalSubscriptionHandle) => {
        handle.close();
      },
    ],
    [
      "loses continuity",
      (handle: LogicalSubscriptionHandle) => {
        handle.continuityLost();
      },
    ],
    [
      "fails presentation",
      (handle: LogicalSubscriptionHandle) => {
        handle.presentationFailed();
      },
    ],
  ])("stops applying held records once an earlier one %s", async (_, effect) => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    let handle: LogicalSubscriptionHandle | null = null;
    const envelopes = vi.fn(() => {
      if (handle !== null) effect(handle);
    });
    handle = pool.subscribe(subscription, sink(envelopes));
    only(sources[0]).open();
    only(sources[0]).emit(heartbeat(1, 1));
    only(sources[0]).emit(heartbeat(1, 2));
    only(sources[0]).acknowledge(subscription);
    await eventLoopBarrier();
    expect(envelopes.mock.calls).toEqual([[heartbeat(1, 1)]]);
  });

  it("drops held records when a stale commit leaves the membership unauthenticated", async () => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    const envelopes = vi.fn();
    const commit = vi.fn(() => "stale" as const);
    pool.subscribe(
      subscription,
      sink(envelopes),
      Object.freeze({ commit, discard: () => undefined, proof: null, subscription }),
    );
    only(sources[0]).open();
    only(sources[0]).emit(heartbeat(1, 1));
    only(sources[0]).acknowledge(subscription);
    await eventLoopBarrier();
    expect(commit).toHaveBeenCalledOnce();
    expect(envelopes).not.toHaveBeenCalled();
    // No control is settling now, so unauthenticated traffic fails closed.
    only(sources[0]).emit(heartbeat(1, 2));
    expect(only(sources[0]).close).toHaveBeenCalledOnce();
    expect(envelopes).not.toHaveBeenCalled();
  });
});

describe("SSE records around a degraded lane's successor", () => {
  it("discards a degraded lane's record while no successor control is settling", async () => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    const successor = authorized(1, { descriptorBinding: "binding-1-successor" });
    const envelopes = vi.fn();
    const handle = pool.subscribe(subscription, sink(envelopes, vi.fn(), successor));
    const source = only(sources[0]);
    source.open();
    source.acknowledge(subscription);
    await eventLoopBarrier();

    handle.presentationFailed();
    // The successor is still being authorized: this is the old lane's record.
    source.emit(heartbeat(1, 1));
    await eventLoopBarrier();
    source.acknowledge(successor);
    await eventLoopBarrier();
    source.emit(heartbeat(1, 2));
    expect(envelopes.mock.calls).toEqual([[heartbeat(1, 2)]]);
    expect(source.close).not.toHaveBeenCalled();
  });

  it("holds the successor's first record when it overtakes the successor's answer", async () => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    const successor = authorized(1, { descriptorBinding: "binding-1-successor" });
    const envelopes = vi.fn();
    const handle = pool.subscribe(subscription, sink(envelopes, vi.fn(), successor));
    const source = only(sources[0]);
    source.open();
    source.acknowledge(subscription);
    await eventLoopBarrier();
    source.emit(heartbeat(1, 1));

    handle.presentationFailed();
    await eventLoopBarrier();
    // The host committed the successor, and its first record overtook the
    // answer. Discarding it would leave the next record a gap.
    source.emit(heartbeat(1, 2));
    source.acknowledge(successor);
    await eventLoopBarrier();
    source.emit(heartbeat(1, 3));
    expect(envelopes.mock.calls).toEqual([[heartbeat(1, 1)], [heartbeat(1, 2)], [heartbeat(1, 3)]]);
    expect(source.close).not.toHaveBeenCalled();
  });
  it("keeps a degraded lane's fence off a replacement group that reuses its generation", async () => {
    const { pool, sources } = harness();
    const subscription = authorized(1);
    const successor = authorized(1, { descriptorBinding: "binding-1-successor" });
    const envelopes = vi.fn();
    const handle = pool.subscribe(subscription, {
      envelope: envelopes,
      reauthorize: () =>
        Promise.resolve(
          Object.freeze({
            commit: () => "stale" as const,
            discard: () => undefined,
            proof: "authoritative_no_tail" as const,
            subscription: successor,
          }),
        ),
      state: vi.fn(),
    });
    only(sources[0]).open();
    only(sources[0]).acknowledge(subscription);
    await eventLoopBarrier();
    handle.presentationFailed();
    await eventLoopBarrier();

    pool.suspend();
    await pool.resume();
    const replacement = only(sources[1]);
    expect(replacement.request.transportGeneration).toBe(
      only(sources[0]).request.transportGeneration,
    );
    replacement.open();
    // The successor's answer settles without authenticating the membership,
    // so no control is settling when the next record arrives.
    replacement.acknowledge(successor);
    await eventLoopBarrier();
    replacement.emit(heartbeat(1, 1));

    // The old group's fence must not reach this group: the record is
    // unauthenticated traffic and fails closed instead of being discarded.
    // The fence's group identity and its clearing on suspend and close each
    // keep it off on their own; this test fails only when both are gone.
    expect(replacement.close).toHaveBeenCalledOnce();
    expect(envelopes).not.toHaveBeenCalled();
  });
});

describe("overtaking records through the browser's own adapters", () => {
  function nativePool() {
    const timers = new FakeTimers();
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
    }[] = [];
    const natives: {
      close: ReturnType<typeof vi.fn>;
      onmessage?: (event: Readonly<{ data: string }>) => void;
      onopen?: () => void;
      send(value: string): void;
    }[] = [];
    const sent: string[] = [];
    const native = () => {
      const created = { close: vi.fn(), send: (value: string) => sent.push(value) };
      natives.push(created);
      return created;
    };
    const ports = new BrowserAsyncTransportPorts({
      eventSource: native,
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        return new Promise((resolve) => controls.push({ request, resolve }));
      },
      timers: timers.port,
      webSocket: native,
    });
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(8),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports: ports,
    });
    return { controls, natives, pool, sent, timers };
  }

  it("holds a native EventSource record until the HTTP control answers", async () => {
    const { controls, natives, pool } = nativePool();
    const envelopes = vi.fn();
    pool.subscribe(authorized(1), sink(envelopes));
    const native = only(natives[0]);
    native.onopen?.();
    native.onmessage?.({ data: heartbeat(1, 1) });
    native.onmessage?.({ data: heartbeat(1, 2) });
    expect(native.close).not.toHaveBeenCalled();
    const control = only(controls[0]);
    const request = control.request;
    control.resolve(
      Object.freeze({
        connection: request.connection,
        controlNonce: request.controlNonce,
        descriptorBinding: request.subscription.descriptorBinding,
        kind: "authenticated",
        operation: request.operation,
        stream: request.subscription.stream,
        subscriptionId: request.subscription.subscriptionId,
        transportGeneration: request.transportGeneration,
      }),
    );
    await eventLoopBarrier();
    expect(envelopes.mock.calls).toEqual([[heartbeat(1, 1)], [heartbeat(1, 2)]]);
    expect(native.close).not.toHaveBeenCalled();
  });

  it("drops held records when the HTTP control times out", async () => {
    const { natives, pool, timers } = nativePool();
    const envelopes = vi.fn();
    const states = vi.fn();
    pool.subscribe(authorized(1), sink(envelopes, states));
    const native = only(natives[0]);
    native.onopen?.();
    native.onmessage?.({ data: heartbeat(1, 1) });
    timers.flush();
    await eventLoopBarrier();
    expect(envelopes).not.toHaveBeenCalled();
    expect(native.close).toHaveBeenCalled();
    expect(states).toHaveBeenLastCalledWith("degraded");
  });

  it("still fails closed on a WebSocket data frame ahead of its acknowledgment frame", () => {
    const { natives, pool, sent } = nativePool();
    const envelopes = vi.fn();
    const states = vi.fn();
    const subscription = authorized(1, {
      document: Object.freeze({ ...authorized(1).document, transport: "websocket" as const }),
    });
    pool.subscribe(subscription, sink(envelopes, states));
    const native = only(natives[0]);
    native.onopen?.();
    expect(sent).toHaveLength(1);
    native.onmessage?.({ data: heartbeat(1, 1) });
    expect(native.close).toHaveBeenCalledOnce();
    expect(states).toHaveBeenLastCalledWith("degraded");
    expect(envelopes).not.toHaveBeenCalled();
  });
});
