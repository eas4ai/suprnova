import { describe, expect, it, vi } from "vitest";

import {
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type AsyncTransportPorts,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
  type LogicalSubscriptionSink,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";

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
  readonly subscribe = vi.fn();
  readonly unsubscribe = vi.fn();

  constructor(readonly request: DocumentTransportConnectRequest) {}

  open(): void {
    this.request.opened();
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
  reauthorize: LogicalSubscriptionSink["reauthorize"] = (prior) => Promise.resolve(prior),
) {
  return {
    envelope: vi.fn(),
    reauthorize: vi.fn(reauthorize),
    state: vi.fn(),
  } satisfies LogicalSubscriptionSink;
}

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

describe("reviewed reconnect authority", () => {
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

  it("does not reset retry attempts on raw open/drop churn", async () => {
    const { pool, sources, timers } = harness();
    const logical = sink();
    pool.subscribe(authorization(1), logical);

    for (let attempt = 0; attempt < 2; attempt += 1) {
      sources[attempt]?.open();
      sources[attempt]?.fail();
      timers.fire(attempt === 0 ? 50 : 100);
      await settle();
    }
    sources[2]?.open();
    sources[2]?.fail();

    expect(sources).toHaveLength(3);
    expect(timers.pending.size).toBe(0);
    expect(logical.state).toHaveBeenLastCalledWith("degraded");
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
    timers.fire(5_000);
    await resumed;

    expect(hungSignal?.aborted).toBe(true);
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(sources[1]?.subscribe).toHaveBeenCalledTimes(99);
    releaseHung?.(authorization(0, { descriptorBinding: "late-binding" }));
    await settle();
    expect(sources).toHaveLength(2);
  });
});
