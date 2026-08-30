import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import {
  BrowserAsyncTransportPorts,
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type AsyncTransportPorts,
  type BrowserAsyncTransportOptions,
  type DocumentMembershipOutcome,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
  type LogicalSubscriptionSink,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import { eventLoopBarrier } from "./support/event-loop-barrier.js";

function authorization(
  index: number,
  overrides: Partial<AuthorizedLogicalSubscription> = {},
): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: `binding-${String(index)}`,
    document: Object.freeze({
      authorizationScope: "shared-document",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([]),
    expiresAt: 20_000,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 5_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 2,
      maximumDelayMs: 4_000,
      minimumDelayMs: 100,
    }),
    stream: `stream-${String(index)}`,
    subscriptionId: `subscription-${String(index).padStart(3, "0")}`,
    ...overrides,
  });
}

class Source implements EventSourcePort {
  readonly close = vi.fn();
  readonly subscribe = vi.fn<
    (
      subscription: AuthorizedLogicalSubscription,
    ) => DocumentMembershipOutcome | Promise<DocumentMembershipOutcome>
  >((subscription) =>
    Object.freeze({
      descriptorBinding: subscription.descriptorBinding,
      kind: "authenticated" as const,
      stream: subscription.stream,
      subscriptionId: subscription.subscriptionId,
      transportGeneration: this.request.transportGeneration,
    }),
  );
  readonly unsubscribe = vi.fn();

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
}

class Timers {
  readonly pending = new Map<number, { callback: VoidFunction; milliseconds: number }>();
  #next = 0;

  readonly port = {
    clearTimeout: (handle: number) => {
      this.pending.delete(handle);
    },
    timeout: (callback: VoidFunction, milliseconds: number) => {
      this.#next += 1;
      this.pending.set(this.#next, { callback, milliseconds });
      return this.#next;
    },
  };

  fire(milliseconds: number): void {
    const item = [...this.pending].find(([, timer]) => timer.milliseconds === milliseconds);
    if (item === undefined) throw new Error(`timer_not_found:${String(milliseconds)}`);
    this.pending.delete(item[0]);
    item[1].callback();
  }
}

function harness() {
  const sources: Source[] = [];
  const timers = new Timers();
  const transports: AsyncTransportPorts = {
    eventSource(request) {
      const source = new Source(request);
      sources.push(source);
      return source;
    },
    webSocket() {
      throw new Error("unexpected_websocket");
    },
  };
  const pool = new DocumentConnectionPool({
    handshakeScheduler: new OriginHandshakeScheduler(),
    randomness: { number: () => 0.5 },
    reauthorizationTimeoutMs: 5_000,
    timers: timers.port,
    transports,
  });
  return { pool, sources, timers };
}

function sink(
  reauthorize: (
    prior: AuthorizedLogicalSubscription,
    signal: AbortSignal,
  ) => AuthorizedLogicalSubscription | Promise<AuthorizedLogicalSubscription> = (prior) =>
    Promise.resolve(prior),
  proof: "authoritative_no_tail" | "complete_replay" = "authoritative_no_tail",
) {
  return {
    envelope: vi.fn(),
    reauthorize: vi.fn(async (prior: AuthorizedLogicalSubscription, signal: AbortSignal) => {
      const subscription = await reauthorize(prior, signal);
      return Object.freeze({
        commit: () => "committed" as const,
        discard: () => undefined,
        proof,
        subscription,
      });
    }),
    state: vi.fn(),
  } satisfies LogicalSubscriptionSink;
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

async function settle(): Promise<void> {
  await eventLoopBarrier();
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

describe("reviewed reconnect authority", () => {
  it("degrades only the failing logical membership in a shared transport group", () => {
    const { pool, sources, timers } = harness();
    const first = sink();
    const second = sink();
    const firstHandle = pool.subscribe(authorization(1), first);
    const secondHandle = pool.subscribe(authorization(2), second);
    sources[0]?.open();
    firstHandle.continuityProved();
    secondHandle.continuityProved();

    firstHandle.presentationFailed();

    expect(first.state).toHaveBeenLastCalledWith("degraded");
    expect(second.state).toHaveBeenLastCalledWith("current");
    expect(sources).toHaveLength(1);
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).not.toContain(50);
  });

  it("accepts the current exact-membership reauthorization after a presentation failure", async () => {
    const { pool, sources } = harness();
    const successor = authorization(1, { descriptorBinding: "binding-1-successor" });
    const first = sink(() => Promise.resolve(successor));
    const second = sink();
    const firstHandle = pool.subscribe(authorization(1), first);
    const secondHandle = pool.subscribe(authorization(2), second);
    sources[0]?.open();
    firstHandle.continuityProved();
    secondHandle.continuityProved();

    firstHandle.presentationFailed();
    await settle();

    expect(first.reauthorize).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(1);
    expect(sources[0]?.unsubscribe).toHaveBeenCalledExactlyOnceWith("subscription-001");
    expect(sources[0]?.subscribe).toHaveBeenCalledWith(successor);
    expect(first.state).toHaveBeenLastCalledWith("current");
    expect(second.state).toHaveBeenLastCalledWith("current");
    expect(sources[0]?.close).not.toHaveBeenCalled();
  });

  it("quarantines exact degraded-lane frames while successor attachment is pending", async () => {
    const { pool, sources } = harness();
    const successor = authorization(1, { descriptorBinding: "binding-1-successor" });
    const first = sink(() => Promise.resolve(successor));
    const second = sink();
    const firstHandle = pool.subscribe(authorization(1), first);
    const secondHandle = pool.subscribe(authorization(2), second);
    sources[0]?.open();
    firstHandle.continuityProved();
    secondHandle.continuityProved();
    let acknowledge!: (value: DocumentMembershipOutcome) => void;
    const pendingAcknowledgment = new Promise<DocumentMembershipOutcome>((resolve) => {
      acknowledge = resolve;
    });
    sources[0]?.subscribe.mockImplementationOnce(() => pendingAcknowledgment);

    firstHandle.presentationFailed();
    await settle();
    sources[0]?.emit(heartbeat(1, 1));

    expect(first.envelope).not.toHaveBeenCalled();
    expect(first.state).toHaveBeenLastCalledWith("connecting");
    expect(second.state).toHaveBeenLastCalledWith("current");
    expect(sources[0]?.close).not.toHaveBeenCalled();

    acknowledge(
      Object.freeze({
        descriptorBinding: successor.descriptorBinding,
        kind: "authenticated" as const,
        stream: successor.stream,
        subscriptionId: successor.subscriptionId,
        transportGeneration: sources[0]?.request.transportGeneration ?? -1,
      }),
    );
    await settle();
    sources[0]?.emit(heartbeat(1, 2));

    expect(first.state).toHaveBeenLastCalledWith("current");
    expect(first.envelope).toHaveBeenCalledExactlyOnceWith(heartbeat(1, 2));
    expect(second.state).toHaveBeenLastCalledWith("current");
    expect(sources[0]?.close).not.toHaveBeenCalled();
  });

  it("keeps a shared source current when unsubscribe synchronously reenters with a degraded frame", async () => {
    const { pool, sources } = harness();
    const first = sink();
    const second = sink();
    const firstHandle = pool.subscribe(authorization(1), first);
    const secondHandle = pool.subscribe(authorization(2), second);
    sources[0]?.open();
    firstHandle.continuityProved();
    secondHandle.continuityProved();
    sources[0]?.unsubscribe.mockImplementationOnce(() => {
      sources[0]?.emit(heartbeat(1, 1));
    });

    firstHandle.presentationFailed();
    await settle();
    sources[0]?.emit(heartbeat(1, 2));

    expect(first.envelope).toHaveBeenCalledExactlyOnceWith(heartbeat(1, 2));
    expect(first.state).toHaveBeenLastCalledWith("current");
    expect(second.state).toHaveBeenLastCalledWith("current");
    expect(sources[0]?.close).not.toHaveBeenCalled();
    expect(sources).toHaveLength(1);
  });

  it("does not inherit a degraded-lane quarantine across a generation-colliding group replacement", async () => {
    const { pool, sources } = harness();
    const successor = authorization(1, { descriptorBinding: "binding-1-successor" });
    const first = sink(() => Promise.resolve(successor));
    const firstHandle = pool.subscribe(authorization(1), first);
    sources[0]?.open();
    firstHandle.continuityProved();

    firstHandle.presentationFailed();
    pool.suspend();
    await pool.resume();
    expect(sources).toHaveLength(2);
    expect(sources[0]?.request.transportGeneration).toBe(1);
    expect(sources[1]?.request.transportGeneration).toBe(1);
    let acknowledge!: (value: DocumentMembershipOutcome) => void;
    sources[1]?.subscribe.mockImplementationOnce(
      () =>
        new Promise<DocumentMembershipOutcome>((resolve) => {
          acknowledge = resolve;
        }),
    );
    sources[1]?.open();

    sources[0]?.emit(heartbeat(1, 1));
    expect(sources[1]?.close).not.toHaveBeenCalled();
    expect(first.envelope).not.toHaveBeenCalled();

    sources[1]?.emit(heartbeat(1, 1));

    expect(sources[1]?.close).toHaveBeenCalledExactlyOnceWith("transport_replaced");
    expect(first.state).toHaveBeenLastCalledWith("degraded");
    expect(first.envelope).not.toHaveBeenCalled();
    acknowledge(
      Object.freeze({
        descriptorBinding: successor.descriptorBinding,
        kind: "authenticated" as const,
        stream: successor.stream,
        subscriptionId: successor.subscriptionId,
        transportGeneration: 1,
      }),
    );
    await settle();
    expect(first.state).toHaveBeenLastCalledWith("degraded");
    pool.dispose();
  });

  it("accepts only the replacement group's exact successor acknowledgment before resuming traffic", async () => {
    const { pool, sources } = harness();
    const successor = authorization(1, { descriptorBinding: "binding-1-successor" });
    const first = sink(() => Promise.resolve(successor));
    const firstHandle = pool.subscribe(authorization(1), first);
    sources[0]?.open();
    firstHandle.continuityProved();

    firstHandle.presentationFailed();
    pool.suspend();
    await pool.resume();
    let acknowledge!: (value: DocumentMembershipOutcome) => void;
    sources[1]?.subscribe.mockImplementationOnce(
      () =>
        new Promise<DocumentMembershipOutcome>((resolve) => {
          acknowledge = resolve;
        }),
    );
    sources[1]?.open();
    sources[0]?.emit(heartbeat(1, 1));
    expect(first.envelope).not.toHaveBeenCalled();

    acknowledge(
      Object.freeze({
        descriptorBinding: successor.descriptorBinding,
        kind: "authenticated" as const,
        stream: successor.stream,
        subscriptionId: successor.subscriptionId,
        transportGeneration: sources[1]?.request.transportGeneration ?? -1,
      }),
    );
    await settle();
    sources[1]?.emit(heartbeat(1, 1));

    expect(first.state).toHaveBeenLastCalledWith("current");
    expect(first.envelope).toHaveBeenCalledExactlyOnceWith(heartbeat(1, 1));
    expect(sources[1]?.close).not.toHaveBeenCalled();
    pool.dispose();
  });

  it("reauthorizes every ordinary reconnect before opening a replacement transport", async () => {
    const { pool, sources, timers } = harness();
    const rotated = authorization(1, { descriptorBinding: "rotated-binding" });
    const logical = sink((prior, signal) => {
      expect(prior.descriptorBinding).toBe("binding-1");
      expect(signal.aborted).toBe(false);
      return Promise.resolve(rotated);
    });
    pool.subscribe(authorization(1), logical);
    sources[0]?.open();
    sources[0]?.fail();

    timers.fire(50);
    await settle();

    expect(logical.reauthorize).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(2);
    expect(sources[1]?.request.authorization).toEqual(rotated.authorization);
  });

  it("does not reset retry attempts when replacement handshakes drop before proof consumption", async () => {
    const { pool, sources, timers } = harness();
    const logical = sink();
    pool.subscribe(authorization(1), logical);
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();
    sources[1]?.fail();
    timers.fire(100);
    await settle();
    sources[2]?.fail();

    expect(sources).toHaveLength(3);
    expect(timers.pending.size).toBe(0);
    expect(logical.state).toHaveBeenLastCalledWith("degraded");
  });

  it("does not consume a continuity proof when the authenticated membership cannot attach", async () => {
    const { pool, sources, timers } = harness();
    const reconnect = Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 1,
      maximumDelayMs: 100,
      minimumDelayMs: 100,
    });
    const logical = sink();
    pool.subscribe(authorization(1, { reconnect }), logical);
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();

    sources[1]?.subscribe.mockImplementation(() => {
      throw new Error("membership_rejected");
    });
    sources[1]?.open();
    sources[1]?.fail();

    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).not.toContain(50);
    expect(logical.state).toHaveBeenLastCalledWith("degraded");
  });

  it("commits one physical continuity outcome per transport generation", async () => {
    const { pool, sources, timers } = harness();
    const logical = sink();
    const handle = pool.subscribe(authorization(1), logical);
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();
    sources[1]?.open();

    expect(logical.state.mock.calls.filter(([state]) => state === "current")).toHaveLength(1);
    handle.continuityProved();
    handle.continuityProved();
    expect(logical.state.mock.calls.filter(([state]) => state === "current")).toHaveLength(1);
  });

  it("does not revive terminal memberships when a later compatible island connects", () => {
    const { pool, sources } = harness();
    const retired = sink();
    pool.subscribe(authorization(1), retired);
    sources[0]?.open();
    sources[0]?.request.failed("protocol_invalid");

    pool.subscribe(authorization(2), sink());
    expect(sources).toHaveLength(2);
    sources[1]?.open();

    expect(retired.state).toHaveBeenLastCalledWith("degraded");
    expect(sources[1]?.subscribe).toHaveBeenCalledOnce();
    expect(sources[1]?.subscribe).toHaveBeenCalledWith(authorization(2));
  });

  it("fails a heterogeneous document credential group independent of insertion order", () => {
    const orders = [
      ["credential-a-1234", "credential-b-1234"],
      ["credential-b-1234", "credential-a-1234"],
    ] as const;
    for (const [left, right] of orders) {
      const { pool } = harness();
      const first = sink();
      pool.subscribe(
        authorization(1, { authorization: Object.freeze({ credential: left, kind: "bearer" }) }),
        first,
      );

      expect(() =>
        pool.subscribe(
          authorization(2, {
            authorization: Object.freeze({ credential: right, kind: "bearer" }),
          }),
          sink(),
        ),
      ).toThrow("async_transport_authority_conflict");
      expect(first.state).toHaveBeenLastCalledWith("degraded");
    }
  });

  it("uses a commutative strict reconnect policy aggregate", () => {
    const policies = [
      Object.freeze({
        kind: "resume_or_refresh" as const,
        maximumAttempts: 4,
        maximumDelayMs: 2_000,
        minimumDelayMs: 100,
      }),
      Object.freeze({
        kind: "resume_or_refresh" as const,
        maximumAttempts: 2,
        maximumDelayMs: 1_000,
        minimumDelayMs: 500,
      }),
    ] as const;
    for (const order of [policies, [...policies].reverse()] as const) {
      const { pool, sources, timers } = harness();
      pool.subscribe(authorization(1, { reconnect: order[0] }), sink());
      pool.subscribe(authorization(2, { reconnect: order[1] }), sink());
      sources[0]?.open();
      sources[0]?.fail();

      expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(250);
    }
  });

  it("rejects a zero-delay reconnect policy that could spin", () => {
    const { pool } = harness();
    expect(() =>
      pool.subscribe(
        authorization(1, {
          reconnect: Object.freeze({
            kind: "resume_or_refresh",
            maximumAttempts: 2,
            maximumDelayMs: 100,
            minimumDelayMs: 0,
          }),
        }),
        sink(),
      ),
    ).toThrow("async_transport_policy_conflict");
  });

  it("times out one noncooperative resume authority without blocking 99 healthy memberships", async () => {
    const { pool, sources, timers } = harness();
    let releaseHung: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    let hungSignal: AbortSignal | undefined;
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(
        authorization(index),
        sink(
          index === 0
            ? (_prior, signal) => {
                hungSignal = signal;
                return new Promise<AuthorizedLogicalSubscription>((resolve) => {
                  releaseHung = resolve;
                });
              }
            : (prior) => Promise.resolve(prior),
        ),
      );
    }
    sources[0]?.open();
    pool.suspend();

    const resumed = pool.resume();
    await settle();

    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(sources[1]?.subscribe.mock.calls.length).toBeGreaterThan(0);
    expect(sources[1]?.subscribe.mock.calls.length).toBeLessThan(100);

    timers.fire(5_000);
    await resumed;

    expect(hungSignal?.aborted).toBe(true);
    expect(sources).toHaveLength(2);
    expect(sources[1]?.subscribe).toHaveBeenCalledTimes(99);
    releaseHung?.(authorization(0, { descriptorBinding: "late-binding" }));
    await settle();
    expect(sources).toHaveLength(2);
  });

  it("keeps reconnect ownership when a progressive resume transport drops", async () => {
    const { pool, sources, timers } = harness();
    pool.subscribe(
      authorization(1),
      sink(() => new Promise(() => undefined)),
    );
    pool.subscribe(
      authorization(2),
      sink((prior) => Promise.resolve(prior)),
    );
    sources[0]?.open();
    pool.suspend();

    void pool.resume();
    await settle();
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    sources[1]?.fail();

    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(50);
    timers.fire(50);
    await settle();
    expect(sources).toHaveLength(3);
  });

  it("fences a late continuity proof from a superseded transport generation", async () => {
    const { pool, sources, timers } = harness();
    let calls = 0;
    let releaseLate: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    const logical = sink((prior) => {
      calls += 1;
      if (calls !== 1) return Promise.resolve(prior);
      return new Promise((resolve) => {
        releaseLate = resolve;
      });
    });
    pool.subscribe(authorization(1), logical);
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();

    pool.suspend();
    const resumed = pool.resume();
    await settle();
    await resumed;
    sources[1]?.open();
    logical.state.mockClear();

    releaseLate?.(authorization(1, { descriptorBinding: "late-proof" }));
    await settle();

    expect(sources).toHaveLength(2);
    expect(logical.state).not.toHaveBeenCalledWith("current");
  });

  it("keeps compatible arrivals behind an active reconnect backoff", async () => {
    const { pool, sources, timers } = harness();
    pool.subscribe(authorization(1), sink());
    sources[0]?.open();
    sources[0]?.fail();

    pool.subscribe(authorization(2), sink());
    expect(sources).toHaveLength(1);

    timers.fire(50);
    await settle();
    expect(sources).toHaveLength(2);
  });

  it("progressively restores a healthy membership while a peer authority is hung", async () => {
    const { pool, sources, timers } = harness();
    let hungSignal: AbortSignal | undefined;
    pool.subscribe(
      authorization(1),
      sink((_prior, signal) => {
        hungSignal = signal;
        return new Promise(() => undefined);
      }),
    );
    pool.subscribe(
      authorization(2),
      sink((prior) => Promise.resolve(prior)),
    );
    sources[0]?.open();
    sources[0]?.fail();

    timers.fire(50);
    await settle();

    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(sources[1]?.subscribe).toHaveBeenCalledWith(authorization(2));
    expect(hungSignal?.aborted).toBe(false);
  });

  it("queues an arrival during reconnect restoration without stale authority", async () => {
    const { pool, sources, timers } = harness();
    let releaseFirst: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    pool.subscribe(
      authorization(1),
      sink(
        (prior) =>
          new Promise((resolve) => {
            releaseFirst = resolve;
            void prior;
          }),
      ),
    );
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();

    let releaseSecond: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    const second = sink(
      (prior) =>
        new Promise((resolve) => {
          releaseSecond = resolve;
          void prior;
        }),
    );
    pool.subscribe(authorization(2), second);
    await settle();

    expect(sources).toHaveLength(1);
    expect(second.reauthorize).toHaveBeenCalledOnce();
    releaseSecond?.(authorization(2));
    await settle();
    expect(sources).toHaveLength(2);

    releaseFirst?.(authorization(1));
  });

  it("admits a compatible arrival against rotated current recovery authority", async () => {
    const { pool, sources, timers } = harness();
    const originalAuthority = Object.freeze({
      credential: "credential-old-1234",
      kind: "bearer" as const,
    });
    const rotatedAuthority = Object.freeze({
      credential: "credential-new-1234",
      kind: "bearer" as const,
    });
    const first = authorization(1, { authorization: originalAuthority });
    const second = authorization(2, { authorization: originalAuthority });
    const rotatedFirst = authorization(1, { authorization: rotatedAuthority });
    pool.subscribe(
      first,
      sink(() => Promise.resolve(rotatedFirst)),
    );
    pool.subscribe(
      second,
      sink(() => new Promise(() => undefined)),
    );
    sources[0]?.open();
    sources[0]?.fail();
    timers.fire(50);
    await settle();

    const third = authorization(3, { authorization: rotatedAuthority });
    expect(() =>
      pool.subscribe(
        third,
        sink(() => Promise.resolve(third)),
      ),
    ).not.toThrow();
    await settle();

    expect(sources).toHaveLength(2);
    expect(sources[1]?.request.authorization).toEqual(rotatedAuthority);
  });
});

describe("reviewed SSE membership ownership", () => {
  it.each([
    { lifecycle: "ordinary reconnect", proof: "authoritative_no_tail" as const },
    { lifecycle: "ordinary reconnect", proof: "complete_replay" as const },
    { lifecycle: "bfcache resume", proof: "authoritative_no_tail" as const },
    { lifecycle: "bfcache resume", proof: "complete_replay" as const },
  ])(
    "waits for real SSE membership acknowledgment after $lifecycle with $proof",
    async ({ lifecycle, proof }) => {
      const timers = new Timers();
      const native: {
        close: ReturnType<typeof vi.fn>;
        onerror?: VoidFunction;
        onopen?: VoidFunction;
      }[] = [];
      const controls: {
        reject(reason?: unknown): void;
        resolve(): void;
        signal: AbortSignal;
      }[] = [];
      const transports = new BrowserAsyncTransportPorts({
        eventSource() {
          const source = { close: vi.fn() };
          native.push(source);
          return source;
        },
        fetch: vi.fn<typeof globalThis.fetch>(),
        membershipTimeoutMs: 5_000,
        sseMembership(request) {
          return new Promise((resolve, reject) => {
            controls.push({
              reject,
              resolve: () => {
                resolve(sseAcknowledgment(request));
              },
              signal: request.signal,
            });
          });
        },
        timers: timers.port,
        webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
      });
      const pool = new DocumentConnectionPool({
        handshakeScheduler: new OriginHandshakeScheduler(),
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      });
      const reconnect = Object.freeze({
        kind: "resume_or_refresh" as const,
        maximumAttempts: 1,
        maximumDelayMs: 100,
        minimumDelayMs: 100,
      });
      const logical = sink((prior) => Promise.resolve(prior), proof);
      pool.subscribe(authorization(1, { reconnect }), logical);
      native[0]?.onopen?.();
      controls[0]?.resolve();
      await settle();

      if (lifecycle === "bfcache resume") {
        pool.suspend();
        const resumed = pool.resume();
        await settle();
        await resumed;
      } else {
        native[0]?.onerror?.();
        timers.fire(50);
        await settle();
      }

      const replacement = native[1];
      logical.state.mockClear();
      replacement?.onopen?.();
      expect(logical.state).not.toHaveBeenCalledWith("current");
      controls[1]?.resolve();
      await settle();
      expect(logical.state.mock.calls.filter(([state]) => state === "current")).toHaveLength(1);

      replacement?.onerror?.();
      expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(50);
    },
  );

  it.each(["reject", "timeout", "transport_lost"] as const)(
    "never consumes real SSE continuity proof when membership control ends with %s",
    async (ending) => {
      const timers = new Timers();
      const native: {
        close: ReturnType<typeof vi.fn>;
        onerror?: VoidFunction;
        onopen?: VoidFunction;
      }[] = [];
      const controls: {
        reject(reason?: unknown): void;
        resolve(): void;
        signal: AbortSignal;
      }[] = [];
      const transports = new BrowserAsyncTransportPorts({
        eventSource() {
          const source = { close: vi.fn() };
          native.push(source);
          return source;
        },
        fetch: vi.fn<typeof globalThis.fetch>(),
        membershipTimeoutMs: 5_000,
        sseMembership(request) {
          return new Promise((resolve, reject) => {
            controls.push({
              reject,
              resolve: () => {
                resolve(sseAcknowledgment(request));
              },
              signal: request.signal,
            });
          });
        },
        timers: timers.port,
        webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
      });
      const pool = new DocumentConnectionPool({
        handshakeScheduler: new OriginHandshakeScheduler(),
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      });
      const logical = sink();
      pool.subscribe(authorization(1), logical);
      native[0]?.onopen?.();
      controls[0]?.resolve();
      await settle();
      native[0]?.onerror?.();
      timers.fire(50);
      await settle();
      logical.state.mockClear();
      native[1]?.onopen?.();

      if (ending === "reject") controls[1]?.reject(new Error("membership_rejected"));
      else if (ending === "timeout") timers.fire(5_000);
      else native[1]?.onerror?.();
      await settle();
      controls[1]?.resolve();
      await settle();

      expect(logical.state).not.toHaveBeenCalledWith("current");
      if (ending === "timeout") expect(controls[1]?.signal.aborted).toBe(true);
    },
  );

  it("admits E100 through one real adapter with bounded membership-control concurrency", async () => {
    const timers = new Timers();
    const native: { close: ReturnType<typeof vi.fn>; onopen?: VoidFunction }[] = [];
    const releases: VoidFunction[] = [];
    let active = 0;
    let maximumActive = 0;
    let started = 0;
    const transports = new BrowserAsyncTransportPorts({
      eventSource() {
        const source = { close: vi.fn() };
        native.push(source);
        return source;
      },
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        started += 1;
        active += 1;
        maximumActive = Math.max(maximumActive, active);
        return new Promise((resolve) => {
          releases.push(() => {
            active -= 1;
            resolve(sseAcknowledgment(request));
          });
        });
      },
      timers: timers.port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports,
    });
    for (let index = 0; index < 100; index += 1) {
      pool.subscribe(
        authorization(index),
        sink((prior) => Promise.resolve(prior)),
      );
    }
    expect(native).toHaveLength(1);
    native[0]?.onopen?.();

    for (let turn = 0; turn < 20 && started < 100; turn += 1) {
      const batch = releases.splice(0);
      for (const release of batch) release();
      await settle();
    }
    for (const release of releases.splice(0)) release();
    await settle();

    expect(started).toBe(100);
    expect(maximumActive).toBeLessThanOrEqual(8);
    expect(active).toBe(0);
    expect(native).toHaveLength(1);
  });
});

describe("reviewed pre-authentication recovery", () => {
  it.each([
    { failure: "pre_open_loss", proof: null, transport: "sse" as const },
    { failure: "never_open", proof: "authoritative_no_tail" as const, transport: "sse" as const },
    { failure: "pre_open_loss", proof: "complete_replay" as const, transport: "sse" as const },
    { failure: "pre_open_loss", proof: null, transport: "websocket" as const },
    {
      failure: "never_open",
      proof: "authoritative_no_tail" as const,
      transport: "websocket" as const,
    },
    {
      failure: "pre_open_loss",
      proof: "complete_replay" as const,
      transport: "websocket" as const,
    },
  ])(
    "preserves an inert $proof initial stage across $transport $failure",
    async ({ failure, proof, transport }) => {
      const sources: Source[] = [];
      const timers = new Timers();
      const transports: AsyncTransportPorts = {
        eventSource(request) {
          if (transport !== "sse") throw new Error("unexpected_sse");
          const source = new Source(request);
          sources.push(source);
          return source;
        },
        webSocket(request) {
          if (transport !== "websocket") throw new Error("unexpected_websocket");
          const source = new Source(request);
          sources.push(source);
          return source;
        },
      };
      const pool = new DocumentConnectionPool({
        handshakeScheduler: new OriginHandshakeScheduler(),
        handshakeTimeoutMs: 5_000,
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      });
      const initial = authorization(1, {
        document: Object.freeze({
          authorizationScope: "shared-document",
          origin: "https://app.example.test",
          transport,
        }),
      });
      const commit = vi.fn(() => "committed" as const);
      const discard = vi.fn();
      const logical = sink(() => Promise.reject(new Error("initial_stage_must_survive")));
      pool.subscribe(
        initial,
        logical,
        Object.freeze({ commit, discard, proof, subscription: initial }),
      );

      if (failure === "never_open") timers.fire(5_000);
      else sources[0]?.fail();
      expect(commit).not.toHaveBeenCalled();
      expect(discard).not.toHaveBeenCalled();
      expect(logical.reauthorize).not.toHaveBeenCalled();

      timers.fire(50);
      await settle();
      expect(sources).toHaveLength(2);
      sources[0]?.open();
      expect(commit).not.toHaveBeenCalled();
      sources[1]?.open();
      await settle();

      expect(logical.reauthorize).not.toHaveBeenCalled();
      expect(commit).toHaveBeenCalledOnce();
      expect(discard).not.toHaveBeenCalled();
      if (proof === null) expect(logical.state).not.toHaveBeenCalledWith("current");
      else expect(logical.state).toHaveBeenCalledWith("current");
    },
  );

  it.each([
    {
      field: "credential",
      rotated: authorization(1, {
        authorization: Object.freeze({ credential: "rotated-credential-1234", kind: "bearer" }),
      }),
    },
    {
      field: "document key and transport",
      rotated: authorization(1, {
        document: Object.freeze({
          authorizationScope: "rotated-scope",
          origin: "https://rotated.example.test",
          transport: "websocket" as const,
        }),
      }),
    },
    {
      field: "reconnect policy",
      rotated: authorization(1, {
        reconnect: Object.freeze({
          kind: "resume_or_refresh" as const,
          maximumAttempts: 3,
          maximumDelayMs: 1_600,
          minimumDelayMs: 800,
        }),
      }),
    },
  ])("attaches bfcache restoration with staged effective $field", async ({ rotated }) => {
    const timers = new Timers();
    const sources: Source[] = [];
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports: {
        eventSource(request) {
          const source = new Source(request);
          sources.push(source);
          return source;
        },
        webSocket(request) {
          const source = new Source(request);
          sources.push(source);
          return source;
        },
      },
    });
    const initial = authorization(1);
    const logical = sink(() => Promise.resolve(rotated));
    pool.subscribe(initial, logical);
    sources[0]?.open();
    pool.suspend();
    await pool.resume();

    expect(sources).toHaveLength(2);
    expect(sources[1]?.request.key).toEqual(rotated.document);
    expect(sources[1]?.request.authorization).toEqual(rotated.authorization);
    expect(sources[1]?.request.key.transport).toBe(rotated.document.transport);
    sources[1]?.open();
    sources[1]?.fail();
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(
      Math.floor(rotated.reconnect.minimumDelayMs * 0.5),
    );
  });

  it("accepts only one terminal transport callback per physical generation", async () => {
    const timers = new Timers();
    const sources: Source[] = [];
    const reconnect = Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 2,
      maximumDelayMs: 400,
      minimumDelayMs: 100,
    });
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports: {
        eventSource(request) {
          const source = new Source(request);
          source.close.mockImplementation(() => {
            request.failed("transport_lost");
            request.failed("transport_lost");
          });
          sources.push(source);
          return source;
        },
        webSocket() {
          throw new Error("unexpected_websocket");
        },
      },
    });
    const logical = sink();
    pool.subscribe(authorization(1, { reconnect }), logical);
    sources[0]?.open();
    sources[0]?.fail();
    sources[0]?.fail();

    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toEqual([50]);
    timers.fire(50);
    await settle();
    expect(sources).toHaveLength(2);
    expect(logical.reauthorize).toHaveBeenCalledOnce();
  });

  it("closes and never retains an adapter that fails synchronously during creation", () => {
    const timers = new Timers();
    const sources: Source[] = [];
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports: {
        eventSource(request) {
          const source = new Source(request);
          sources.push(source);
          request.failed("transport_lost");
          return source;
        },
        webSocket() {
          throw new Error("unexpected_websocket");
        },
      },
    });
    pool.subscribe(authorization(1), sink());

    expect(sources).toHaveLength(1);
    expect(sources[0]?.close).toHaveBeenCalledOnce();
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toEqual([50]);
    pool.dispose();
    expect(timers.pending.size).toBe(0);
  });

  it.each([{ completionOrder: [0, 1] as const }, { completionOrder: [1, 0] as const }] as const)(
    "groups compatible staged bfcache rotations independently of completion order $completionOrder",
    async ({ completionOrder }) => {
      const timers = new Timers();
      const sources: Source[] = [];
      const rotatedAuthority = Object.freeze({
        credential: "rotated-compatible-credential",
        kind: "bearer" as const,
      });
      const releases: (((value: AuthorizedLogicalSubscription) => void) | undefined)[] = [];
      const pool = new DocumentConnectionPool({
        handshakeScheduler: new OriginHandshakeScheduler(),
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new Source(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      });
      for (const index of [0, 1]) {
        const current = authorization(index);
        pool.subscribe(
          current,
          sink(
            () =>
              new Promise((resolve) => {
                releases[index] = resolve;
              }),
          ),
        );
      }
      sources[0]?.open();
      pool.suspend();
      const resumed = pool.resume();
      await settle();
      for (const index of completionOrder) {
        releases[index]?.(
          authorization(index, {
            authorization: rotatedAuthority,
            descriptorBinding: `rotated-binding-${String(index)}`,
          }),
        );
        await settle();
      }
      await resumed;

      expect(sources).toHaveLength(2);
      expect(sources[1]?.request.authorization).toEqual(rotatedAuthority);
      sources[1]?.open();
      expect(sources[1]?.subscribe).toHaveBeenCalledTimes(2);
      pool.dispose();
    },
  );

  it.each([
    { credentials: ["credential-a", "credential-b"] as const },
    { credentials: ["credential-b", "credential-a"] as const },
  ] as const)(
    "rejects incompatible staged bfcache authority independent of completion order $credentials",
    async ({ credentials }) => {
      const timers = new Timers();
      const sources: Source[] = [];
      const states = [vi.fn(), vi.fn()] as const;
      const pool = new DocumentConnectionPool({
        handshakeScheduler: new OriginHandshakeScheduler(),
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new Source(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      });
      for (const index of [0, 1] as const) {
        pool.subscribe(authorization(index), {
          ...sink(() =>
            Promise.resolve(
              authorization(index, {
                authorization: Object.freeze({
                  credential: credentials[index],
                  kind: "bearer" as const,
                }),
              }),
            ),
          ),
          state: states[index],
        });
      }
      sources[0]?.open();
      pool.suspend();
      await pool.resume();

      expect(states[0]).toHaveBeenCalledWith("degraded");
      expect(states[1]).toHaveBeenCalledWith("degraded");
      pool.dispose();
    },
  );
});
