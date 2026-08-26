import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import type { JsonValue } from "../src/canonical.js";
import {
  AsyncDocumentOwner,
  type AsyncAuthorizationRequest,
} from "../src/async-updates/feature.js";
import type {
  AsyncTransportPorts,
  DocumentTransportConnectRequest,
  EventSourcePort,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  RuntimeFeatureDirectiveOwnership,
  RuntimeFeatureDocumentContext,
  RuntimeFeatureIslandPort,
} from "../src/features/contract.js";

function authorization(sequence: bigint): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence }),
    descriptorBinding: `binding-${String(sequence)}`,
    document: Object.freeze({
      authorizationScope: "document-scope",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([
      Object.freeze({
        maximumFanout: 4,
        name: "orders.updated",
        schema: "json" as const,
        targets: Object.freeze(["self"]),
        version: 1,
      }),
    ]),
    expiresAt: 20_000,
    heartbeatTimeoutMs: 5_000,
    presentationSignals: Object.freeze([
      Object.freeze({ name: "completion_percent", schema: "u64" as const }),
    ]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 4_000,
      minimumDelayMs: 100,
    }),
    stream: "orders",
    subscriptionId: "subscription-001",
  });
}

function envelope(sequence: bigint, payload: JsonValue): string {
  return canonicalize({
    payload,
    position: { epoch: "1", sequence: String(sequence) },
    protocol_version: 1,
    stream: "orders",
    subscription: "subscription-001",
  });
}

class FakeSource implements EventSourcePort {
  readonly close = vi.fn();
  readonly subscribe = vi.fn();
  readonly unsubscribe = vi.fn();

  constructor(readonly request: DocumentTransportConnectRequest) {}

  open(): void {
    this.request.opened();
  }

  emit(encoded: string): void {
    this.request.message(encoded);
  }
}

class FakeTimers {
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
    const found = [...this.pending].find(([, timer]) => timer.milliseconds === milliseconds);
    if (found === undefined) throw new Error("timer_not_found");
    this.pending.delete(found[0]);
    found[1].callback();
  }
}

function ownership(root: Element): RuntimeFeatureDirectiveOwnership {
  return Object.freeze({
    attributeName: "live:stream",
    directive: Object.freeze({
      capability: "async@1" as const,
      modifiers: Object.freeze([]),
      name: "stream",
      ok: true as const,
      role: null,
      value: "orders",
    }),
    element: root,
  });
}

describe("async feature lifecycle", () => {
  it("reauthorizes after bfcache, proves continuity, and ignores retired transport data", async () => {
    const sources: FakeSource[] = [];
    const transports: AsyncTransportPorts = {
      eventSource(request) {
        const source = new FakeSource(request);
        sources.push(source);
        return source;
      },
      webSocket() {
        throw new Error("unexpected_websocket");
      },
    };
    const timers = new FakeTimers();
    const authorizationRequests: AsyncAuthorizationRequest[] = [];
    const root = Object.freeze({}) as Element;
    const refresh = vi.fn(() => "queued" as const);
    const event = vi.fn(() => "dispatched" as const);
    const signal = vi.fn((_element: Element, _name: string, value: JsonValue) => value);
    const port: RuntimeFeatureIslandPort = {
      dispatchRegisteredEvent: event,
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: () => "accepted",
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: signal,
    };
    const context: RuntimeFeatureDocumentContext = {
      diagnose: vi.fn(),
      onDispose: vi.fn(),
    };
    const owner = new AsyncDocumentOwner(context, {
      authority: {
        authorize(request) {
          authorizationRequests.push(request);
          const current = authorization(request.position?.sequence ?? 0n);
          return Promise.resolve(
            request.prior === null
              ? current
              : Object.freeze({
                  replay: Object.freeze([envelope(4n, { kind: "refresh", name: "refresh" })]),
                  subscription: current,
                }),
          );
        },
      },
      clock: { now: () => 1_000 },
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports,
    });

    const controller = owner.connectIsland(port);
    await Promise.resolve();
    await Promise.resolve();
    expect(sources).toHaveLength(1);
    sources[0]?.open();
    sources[0]?.emit(envelope(1n, { kind: "refresh", name: "refresh" }));
    sources[0]?.emit(
      envelope(2n, { kind: "presentation_signal", name: "completion_percent", value: 75 }),
    );
    sources[0]?.emit(
      envelope(3n, {
        event: "orders.updated",
        kind: "browser_event",
        payload: { order: 42 },
        schema_version: 1,
        target: "self",
      }),
    );

    expect(refresh).toHaveBeenCalledOnce();
    expect(signal).toHaveBeenCalledWith(root, "completion_percent", 75);
    expect(event).toHaveBeenCalledWith({
      event: "orders.updated",
      maximumFanout: 4,
      payload: { order: 42 },
      schemaVersion: 1,
      target: "self",
    });
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(5_000);

    owner.suspend();
    expect(sources[0]?.close).toHaveBeenCalled();
    expect(timers.pending.size).toBe(0);
    await owner.resume();
    expect(authorizationRequests[authorizationRequests.length - 1]?.position).toEqual({
      epoch: 1n,
      sequence: 3n,
    });
    expect(sources).toHaveLength(2);
    expect(refresh).toHaveBeenCalledTimes(2);
    sources[1]?.open();
    sources[0]?.emit(envelope(4n, { kind: "refresh", name: "refresh" }));
    expect(refresh).toHaveBeenCalledTimes(2);
    sources[1]?.emit(envelope(5n, { kind: "refresh", name: "refresh" }));
    expect(refresh).toHaveBeenCalledTimes(3);

    timers.fire(5_000);
    expect(sources[1]?.close).toHaveBeenCalled();
    controller.dispose();
    owner.dispose();
  });

  it("cancels pending authorization when an island retires", async () => {
    let resolveAuthorization: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    const root = Object.freeze({}) as Element;
    const source = vi.fn();
    const port = {
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: vi.fn(() => "accepted" as const),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: vi.fn((_element: Element, _name: string, value: JsonValue) => value),
    } satisfies RuntimeFeatureIslandPort;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            new Promise((resolve) => {
              resolveAuthorization = resolve;
            }),
        },
        clock: { now: () => 1_000 },
        randomness: { number: () => 0.5 },
        timers: new FakeTimers().port,
        transports: { eventSource: source, webSocket: source },
      },
    );
    const controller = owner.connectIsland(port);
    controller.dispose();
    resolveAuthorization?.(authorization(0n));
    await Promise.resolve();

    expect(source).not.toHaveBeenCalled();
    owner.dispose();
  });

  it("cancels an in-progress pageshow reauthorization when the page suspends again", async () => {
    let resolveResume: ((value: AuthorizedLogicalSubscription) => void) | undefined;
    let authorizations = 0;
    const sources: FakeSource[] = [];
    const transports: AsyncTransportPorts = {
      eventSource(request) {
        const source = new FakeSource(request);
        sources.push(source);
        return source;
      },
      webSocket() {
        throw new Error("unexpected_websocket");
      },
    };
    const root = Object.freeze({}) as Element;
    const port = {
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: vi.fn(() => "accepted" as const),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: vi.fn((_element: Element, _name: string, value: JsonValue) => value),
    } satisfies RuntimeFeatureIslandPort;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize() {
            authorizations += 1;
            if (authorizations === 1) return Promise.resolve(authorization(0n));
            return new Promise((resolve) => {
              resolveResume = resolve;
            });
          },
        },
        clock: { now: () => 1_000 },
        randomness: { number: () => 0.5 },
        timers: new FakeTimers().port,
        transports,
      },
    );
    owner.connectIsland(port);
    await Promise.resolve();
    await Promise.resolve();
    expect(sources).toHaveLength(1);
    sources[0]?.open();
    owner.suspend();

    const resumed = owner.resume();
    await Promise.resolve();
    owner.suspend();
    resolveResume?.(authorization(0n));
    await resumed;

    expect(sources).toHaveLength(1);
    owner.dispose();
  });
});
