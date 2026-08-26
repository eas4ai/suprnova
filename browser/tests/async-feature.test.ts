import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import type { JsonValue } from "../src/canonical.js";
import {
  AsyncDocumentOwner,
  type AsyncAuthorizationRequest,
} from "../src/async-updates/feature.js";
import {
  DocumentConnectionPool,
  type AsyncTransportPorts,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  RuntimeFeatureDirectiveOwnership,
  RuntimeFeatureDocumentContext,
  RuntimeFeatureIslandPort,
} from "../src/features/contract.js";

function authorization(
  sequence: bigint,
  overrides: Partial<AuthorizedLogicalSubscription> = {},
): AuthorizedLogicalSubscription {
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
        cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
        maximumFanout: 4,
        name: "orders.updated",
        order: "per_source_sequence" as const,
        payloadContract: "orders.updated.v1",
        schema: "json" as const,
        source: "stream" as const,
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
    ...overrides,
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

function eventCapability(): ReturnType<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]> {
  return Object.freeze({}) as ReturnType<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]>;
}

describe("async feature lifecycle", () => {
  it("does not report physical continuity for a duplicate while the logical stream is connecting", async () => {
    const sources: FakeSource[] = [];
    const continuityProved = vi.fn();
    // eslint-disable-next-line @typescript-eslint/unbound-method -- captured before the temporary prototype spy and invoked with an explicit pool receiver
    const originalSubscribe = DocumentConnectionPool.prototype.subscribe;
    const subscribe = vi
      .spyOn(DocumentConnectionPool.prototype, "subscribe")
      .mockImplementation(function (this: DocumentConnectionPool, authorized, sink) {
        const handle = originalSubscribe.call(this, authorized, sink);
        return Object.freeze({
          close: () => {
            handle.close();
          },
          continuityProved: () => {
            continuityProved();
            handle.continuityProved();
          },
          heartbeatLost: () => {
            handle.heartbeatLost();
          },
        });
      });
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: { authorize: () => authorization(0n) },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: new FakeTimers().port,
        transports: {
          eventSource(request) {
            const source = new FakeSource(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      },
    );
    try {
      owner.connectIsland({
        authorizeRegisteredEvents: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: () => "queued",
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: "document-duplicate",
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        proposeUploadHandle: () => "accepted",
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: (_element, _name, value) => value,
      });
      await Promise.resolve();
      await Promise.resolve();
      sources[0]?.open();

      sources[0]?.emit(envelope(0n, { kind: "heartbeat" }));
      expect(continuityProved).not.toHaveBeenCalled();

      sources[0]?.emit(envelope(1n, { kind: "heartbeat" }));
      expect(continuityProved).toHaveBeenCalledOnce();
    } finally {
      owner.dispose();
      subscribe.mockRestore();
    }
  });

  it("applies a complete initial replay from the signed baseline before transport delivery", async () => {
    const sources: FakeSource[] = [];
    const refresh = vi.fn(() => "queued" as const);
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            Object.freeze({
              replay: Object.freeze([envelope(1n, { kind: "refresh", name: "refresh" })]),
              subscription: authorization(0n),
            }),
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: new FakeTimers().port,
        transports: {
          eventSource(request) {
            const source = new FakeSource(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      },
    );
    owner.connectIsland({
      authorizeRegisteredEvents: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-initial-replay",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: () => "accepted",
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(refresh).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(1);
    owner.dispose();
  });

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
      authorizeRegisteredEvents: eventCapability,
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
    expect(event).toHaveBeenCalledWith(expect.any(Object), {
      event: "orders.updated",
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
      authorizeRegisteredEvents: eventCapability,
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

  it("atomically uses rotated event authority and heartbeat policy after reauthorization", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const initial = authorization(0n);
    const initialEvent = initial.events[0];
    if (initialEvent === undefined) throw new Error("missing_event_fixture");
    const rotated = authorization(0n, {
      descriptorBinding: "binding-rotated",
      events: Object.freeze([Object.freeze({ ...initialEvent, maximumFanout: 1, version: 2 })]),
      heartbeatTimeoutMs: 777,
    });
    let calls = 0;
    const capabilities: object[] = [];
    const authorizeRegisteredEvents = vi.fn(() => {
      const capability = Object.freeze({});
      capabilities.push(capability);
      return capability as ReturnType<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]>;
    });
    const dispatch = vi.fn<RuntimeFeatureIslandPort["dispatchRegisteredEvent"]>(
      () => "dispatched" as const,
    );
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize() {
            calls += 1;
            return calls === 1
              ? initial
              : Object.freeze({ replay: Object.freeze([]), subscription: rotated });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new FakeSource(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      },
    );
    owner.connectIsland({
      authorizeRegisteredEvents,
      dispatchRegisteredEvent: dispatch,
      element: root,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-rotation",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: () => "accepted",
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    owner.suspend();
    await owner.resume();
    sources[1]?.open();
    sources[1]?.emit(
      envelope(1n, {
        event: "orders.updated",
        kind: "browser_event",
        payload: { order: 84 },
        schema_version: 2,
        target: "self",
      }),
    );

    expect(authorizeRegisteredEvents).toHaveBeenLastCalledWith(
      expect.objectContaining({
        descriptorBinding: "binding-rotated",
        events: [expect.objectContaining({ maximumFanout: 1, version: 2 })],
      }),
    );
    expect(dispatch.mock.calls[0]?.[0]).toBe(capabilities[1]);
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(777);
    owner.dispose();
  });

  it("recovers an ordinary reconnect tail before opening the replacement transport", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const requests: AsyncAuthorizationRequest[] = [];
    const refresh = vi.fn(() => "queued" as const);
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            requests.push(request);
            const current = authorization(request.position?.sequence ?? 0n);
            return request.prior === null
              ? current
              : Object.freeze({
                  replay: Object.freeze([envelope(2n, { kind: "refresh", name: "refresh" })]),
                  subscription: current,
                });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new FakeSource(request);
            sources.push(source);
            return source;
          },
          webSocket() {
            throw new Error("unexpected_websocket");
          },
        },
      },
    );
    owner.connectIsland({
      authorizeRegisteredEvents: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-reconnect",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: () => "accepted",
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    sources[0]?.emit(envelope(1n, { kind: "heartbeat" }));
    sources[0]?.request.failed("transport_lost");
    timers.fire(50);
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();

    expect(requests[1]?.position).toEqual({ epoch: 1n, sequence: 1n });
    expect(refresh).toHaveBeenCalledOnce();
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    sources[1]?.emit(envelope(3n, { kind: "refresh", name: "refresh" }));
    expect(refresh).toHaveBeenCalledTimes(2);
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
      authorizeRegisteredEvents: eventCapability,
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
