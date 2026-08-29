import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import type { JsonValue } from "../src/canonical.js";
import {
  AsyncDocumentOwner,
  type AsyncAuthorizationRequest,
  type AsyncQueuePressureObservation,
} from "../src/async-updates/feature.js";
import {
  BrowserAsyncTransportPorts,
  DocumentConnectionPool,
  type AsyncTransportPorts,
  type BrowserAsyncTransportOptions,
  type DocumentMembershipOutcome,
  type DocumentTransportConnectRequest,
  type EventSourcePort,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  RuntimeFeatureDirectiveOwnership,
  RuntimeFeatureDocumentContext,
  AsyncRuntimeIslandPort,
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
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 5_000,
    presentationSignals: Object.freeze([
      Object.freeze({ name: "completion_percent", schema: "u64" as const, scope: "root-scope" }),
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
  readonly subscribe = vi.fn(
    (
      subscription: AuthorizedLogicalSubscription,
    ): DocumentMembershipOutcome | Promise<DocumentMembershipOutcome> =>
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

function successfulFreshRender(
  _reason: unknown,
  completion?: (outcome: "succeeded" | "failed" | "canceled" | "retired") => void,
): "queued" {
  completion?.("succeeded");
  return "queued";
}

function immediateHybridOwnership(root: Element): readonly RuntimeFeatureDirectiveOwnership[] {
  return Object.freeze([
    ownership(root),
    Object.freeze({
      attributeName: "live:poll.immediate",
      directive: Object.freeze({
        capability: "async@1" as const,
        modifiers: Object.freeze(["immediate"]),
        name: "poll",
        ok: true as const,
        role: null,
        value: "",
      }),
      element: root,
    }),
  ]);
}

function pushOnlyOwnership(root: Element): RuntimeFeatureDirectiveOwnership {
  return Object.freeze({
    attributeName: "live:stream.push-only",
    directive: Object.freeze({
      capability: "async@1" as const,
      modifiers: Object.freeze(["push-only"]),
      name: "stream",
      ok: true as const,
      role: null,
      value: "orders",
    }),
    element: root,
  });
}

function pollOwnership(
  root: Element,
  modifiers: readonly string[],
): RuntimeFeatureDirectiveOwnership {
  return Object.freeze({
    attributeName: `live:poll${modifiers.map((modifier) => `.${modifier}`).join("")}`,
    directive: Object.freeze({
      capability: "async@1" as const,
      modifiers: Object.freeze([...modifiers]),
      name: "poll",
      ok: true as const,
      role: null,
      value: "",
    }),
    element: root,
  });
}

function eventCapability(): ReturnType<AsyncRuntimeIslandPort["consumeRegisteredEventCapability"]> {
  return Object.freeze({}) as ReturnType<
    AsyncRuntimeIslandPort["consumeRegisteredEventCapability"]
  >;
}

async function flushMicrotasks(turns = 8): Promise<void> {
  for (let turn = 0; turn < turns; turn += 1) await Promise.resolve();
}

describe("async feature lifecycle", () => {
  it("exposes only closed queue counts to the pressure observer", async () => {
    const sources: FakeSource[] = [];
    const observations: AsyncQueuePressureObservation[] = [];
    const timers = new FakeTimers();
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: { authorize: () => authorization(0n) },
        clock: { now: () => 100 },
        observeQueuePressure: (observation) => observations.push(observation),
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-queue-owner",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await flushMicrotasks();
    sources[0]?.open();
    await flushMicrotasks();
    sources[0]?.emit(envelope(1n, { kind: "refresh", name: "refresh" }));

    expect(observations).toContainEqual({
      documentQueuedBytes: 0,
      documentQueuedEvents: 0,
      islandInFlightRefreshes: 1,
      islandQueuedRefreshes: 0,
    });
    const lastObservation = observations[observations.length - 1];
    if (lastObservation === undefined) throw new Error("queue_observation_missing");
    expect(Object.keys(lastObservation).sort()).toEqual([
      "documentQueuedBytes",
      "documentQueuedEvents",
      "islandInFlightRefreshes",
      "islandQueuedRefreshes",
    ]);
    owner.dispose();
  });

  it("proves exact no-tail continuity before an immediate hybrid timer can refresh", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const refresh = vi.fn(successfulFreshRender);
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            Object.freeze({
              replay: Object.freeze([]),
              subscription: authorization(0n, {
                fallbackPoll: Object.freeze({
                  initial: "immediate" as const,
                  intervalMs: 30_000,
                  jitterRatio: 0.2,
                  visibility: "visible" as const,
                }),
              }),
            }),
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-immediate-no-tail",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => immediateHybridOwnership(root),
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(refresh).not.toHaveBeenCalled();
    sources[0]?.open();
    await Promise.resolve();
    await Promise.resolve();

    expect(refresh).not.toHaveBeenCalled();
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      false,
    );
    owner.dispose();
  });

  it.each([
    { scenario: "latest interval", transport: "sse" },
    { scenario: "latest interval", transport: "websocket" },
    { scenario: "removed poll", transport: "sse" },
    { scenario: "removed poll", transport: "websocket" },
    { scenario: "directive conflict", transport: "sse" },
    { scenario: "directive conflict", transport: "websocket" },
  ] as const)(
    "defers $scenario morph intent until the delayed $transport membership proof",
    async ({ scenario, transport }) => {
      const timers = new FakeTimers();
      const refresh = vi.fn(successfulFreshRender);
      const event = vi.fn(() => "dispatched" as const);
      const diagnose = vi.fn();
      const root = Object.freeze({}) as Element;
      let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([
        ownership(root),
      ]);
      const sources: FakeSource[] = [];
      let acknowledgeSse: VoidFunction | undefined;
      const sent: string[] = [];
      const sockets: {
        close: ReturnType<typeof vi.fn>;
        onmessage?: (event?: unknown) => void;
        onopen?: VoidFunction;
        send(data: string): void;
      }[] = [];
      const subscription = authorization(0n, {
        document: Object.freeze({
          authorizationScope: "document-scope",
          origin: "https://app.example.test",
          transport,
        }),
      });
      const replay = Object.freeze([
        envelope(1n, { kind: "refresh", name: "refresh" }),
        envelope(2n, {
          event: "orders.updated",
          kind: "browser_event",
          payload: { order: 42 },
          schema_version: 1,
          target: "self",
        }),
      ]);
      const transports: AsyncTransportPorts =
        transport === "sse"
          ? {
              eventSource(request) {
                const source = new FakeSource(request);
                source.subscribe.mockImplementation(
                  (current) =>
                    new Promise((resolve) => {
                      acknowledgeSse = () => {
                        resolve(
                          Object.freeze({
                            descriptorBinding: current.descriptorBinding,
                            kind: "authenticated" as const,
                            stream: current.stream,
                            subscriptionId: current.subscriptionId,
                            transportGeneration: request.transportGeneration,
                          }),
                        );
                      };
                    }),
                );
                sources.push(source);
                return source;
              },
              webSocket() {
                throw new Error("unexpected_websocket");
              },
            }
          : new BrowserAsyncTransportPorts({
              eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
              fetch: vi.fn<typeof globalThis.fetch>(),
              membershipTimeoutMs: 5_000,
              sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
              timers: timers.port,
              webSocket() {
                const socket = {
                  close: vi.fn(),
                  send(data: string) {
                    sent.push(data);
                  },
                };
                sockets.push(socket);
                return socket;
              },
            });
      const owner = new AsyncDocumentOwner(
        { diagnose, onDispose: vi.fn() },
        {
          authority: {
            authorize: () => Object.freeze({ replay, subscription }),
          },
          clock: { now: () => 100 },
          randomness: { number: () => 0.5 },
          timers: timers.port,
          transports,
        },
      );
      const controller = owner.connectIsland({
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: event,
        element: root,
        enqueueFreshRender: refresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `document-pending-${transport}-${scenario}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () => ownerships,
        writePresentationSignal: (_element, _name, value) => value,
      });
      await flushMicrotasks();
      if (transport === "sse") sources[0]?.open();
      else sockets[0]?.onopen?.();
      await flushMicrotasks();

      controller.beforeMorph?.();
      ownerships = Object.freeze([ownership(root), pollOwnership(root, ["5s", "immediate"])]);
      controller.afterMorph?.();
      if (scenario === "latest interval") {
        controller.beforeMorph?.();
        ownerships = Object.freeze([ownership(root), pollOwnership(root, ["10s", "immediate"])]);
        controller.afterMorph?.();
      } else if (scenario === "removed poll") {
        controller.beforeMorph?.();
        ownerships = Object.freeze([pushOnlyOwnership(root)]);
        controller.afterMorph?.();
      } else {
        controller.beforeMorph?.();
        ownerships = Object.freeze([
          pushOnlyOwnership(root),
          pollOwnership(root, ["5s", "immediate"]),
        ]);
        controller.afterMorph?.();
      }

      expect(refresh).not.toHaveBeenCalled();
      expect(event).not.toHaveBeenCalled();

      if (transport === "sse") {
        acknowledgeSse?.();
      } else {
        const request = JSON.parse(sent[0] ?? "null") as Record<string, unknown>;
        sockets[0]?.onmessage?.({
          data: canonicalize({
            control_nonce: String(request["control_nonce"]),
            descriptor_binding: String(request["descriptor_binding"]),
            kind: "membership_authenticated",
            stream: String(request["stream"]),
            subscription: String(request["subscription"]),
            transport_generation: Number(request["transport_generation"]),
          }),
        });
      }
      await flushMicrotasks();

      if (scenario === "directive conflict") {
        expect(diagnose).toHaveBeenCalledWith("operation_rejected");
        expect(refresh.mock.calls.some(([reason]) => reason === "poll")).toBe(false);
        expect(event).not.toHaveBeenCalled();
      } else {
        expect(refresh).toHaveBeenCalledOnce();
        expect(event).toHaveBeenCalledOnce();
        if (transport === "sse") sources[0]?.emit(envelope(4n, { kind: "heartbeat" }));
        else sockets[0]?.onmessage?.({ data: envelope(4n, { kind: "heartbeat" }) });
        const pendingDelays = [...timers.pending.values()].map(({ milliseconds }) => milliseconds);
        if (scenario === "latest interval") expect(pendingDelays).toContain(11_000);
        else {
          expect(pendingDelays).not.toContain(5_500);
          expect(pendingDelays).not.toContain(33_000);
        }
      }
      owner.dispose();
    },
  );

  it.each(
    (["sse", "websocket"] as const).flatMap((transport) =>
      (["change", "remove", "push-only"] as const).flatMap((scenario) =>
        (["authenticate", "reject", "suspend"] as const).map((ending) => ({
          ending,
          scenario,
          transport,
        })),
      ),
    ),
  )(
    "fences the old fallback for $transport replacement membership, $scenario, and $ending",
    async ({ ending, scenario, transport }) => {
      const timers = new FakeTimers();
      const refresh = vi.fn(() => "queued" as const);
      const freshness: string[] = [];
      const root = Object.freeze({}) as Element;
      let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([
        ownership(root),
        pollOwnership(root, ["5s"]),
      ]);
      const sources: FakeSource[] = [];
      const sseControls: {
        reject(reason?: unknown): void;
        resolve(): void;
      }[] = [];
      const sockets: {
        close: ReturnType<typeof vi.fn>;
        onclose?: VoidFunction;
        onmessage?: (event: Readonly<{ data: string }>) => void;
        onopen?: VoidFunction;
        readonly sent: string[];
        send(data: string): void;
      }[] = [];
      const current = authorization(0n, {
        document: Object.freeze({
          authorizationScope: "document-scope",
          origin: "https://app.example.test",
          transport,
        }),
      });
      const transports: AsyncTransportPorts =
        transport === "sse"
          ? {
              eventSource(request) {
                const source = new FakeSource(request);
                if (sources.length > 0) {
                  source.subscribe.mockImplementation(
                    (subscription) =>
                      new Promise((resolve, reject) => {
                        sseControls.push({
                          reject,
                          resolve: () => {
                            resolve(
                              Object.freeze({
                                descriptorBinding: subscription.descriptorBinding,
                                kind: "authenticated" as const,
                                stream: subscription.stream,
                                subscriptionId: subscription.subscriptionId,
                                transportGeneration: request.transportGeneration,
                              }),
                            );
                          },
                        });
                      }),
                  );
                }
                sources.push(source);
                return source;
              },
              webSocket() {
                throw new Error("unexpected_websocket");
              },
            }
          : new BrowserAsyncTransportPorts({
              eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
              fetch: vi.fn<typeof globalThis.fetch>(),
              membershipTimeoutMs: 5_000,
              sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
              timers: timers.port,
              webSocket() {
                const sent: string[] = [];
                const socket = {
                  close: vi.fn(),
                  send(data: string) {
                    sent.push(data);
                  },
                  sent,
                };
                sockets.push(socket);
                return socket;
              },
            });
      const acknowledgeWebSocket = (index: number): void => {
        const socket = sockets[index];
        const request = JSON.parse(socket?.sent[0] ?? "null") as Record<string, unknown>;
        socket?.onmessage?.({
          data: canonicalize({
            control_nonce: String(request["control_nonce"]),
            descriptor_binding: String(request["descriptor_binding"]),
            kind: "membership_authenticated",
            stream: String(request["stream"]),
            subscription: String(request["subscription"]),
            transport_generation: Number(request["transport_generation"]),
          }),
        });
      };
      const owner = new AsyncDocumentOwner(
        { diagnose: vi.fn(), onDispose: vi.fn() },
        {
          authority: {
            authorize(request) {
              return request.prior === null
                ? current
                : Object.freeze({ replay: Object.freeze([]), subscription: current });
            },
          },
          clock: { now: () => 100 },
          observeFreshness: ({ state }) => freshness.push(state),
          randomness: { number: () => 0.5 },
          timers: timers.port,
          transports,
        },
      );
      const controller = owner.connectIsland({
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: refresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `document-replacement-policy-${transport}-${scenario}-${ending}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () => ownerships,
        writePresentationSignal: (_element, _name, value) => value,
      });
      await flushMicrotasks();
      if (transport === "sse") sources[0]?.open();
      else {
        sockets[0]?.onopen?.();
        acknowledgeWebSocket(0);
      }
      await flushMicrotasks();

      if (transport === "sse") sources[0]?.request.failed("transport_lost");
      else sockets[0]?.onclose?.();
      const oldFallback = [...timers.pending.values()].find(
        ({ milliseconds }) => milliseconds === 5_500,
      );
      expect(oldFallback).toBeDefined();
      timers.fire(50);
      await flushMicrotasks();
      if (transport === "sse") sources[1]?.open();
      else sockets[1]?.onopen?.();
      await flushMicrotasks();

      controller.beforeMorph?.();
      ownerships =
        scenario === "change"
          ? Object.freeze([ownership(root), pollOwnership(root, ["10s"])])
          : scenario === "remove"
            ? Object.freeze([ownership(root)])
            : Object.freeze([pushOnlyOwnership(root)]);
      controller.afterMorph?.();

      expect(
        [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 5_500),
      ).toHaveLength(0);
      oldFallback?.callback();
      expect(refresh).not.toHaveBeenCalled();
      expect(
        [...timers.pending.values()].some(
          ({ milliseconds }) => milliseconds === 11_000 || milliseconds === 33_000,
        ),
      ).toBe(false);

      let activeTransport = 1;
      if (ending === "suspend") {
        freshness.length = 0;
        owner.suspend();
        expect(freshness).toEqual(["suspended"]);
        expect(timers.pending.size).toBe(0);
        expect(refresh).not.toHaveBeenCalled();
        if (transport === "sse") sseControls[0]?.resolve();
        else acknowledgeWebSocket(1);
        await flushMicrotasks();
        expect(freshness).toEqual(["suspended"]);
        expect(timers.pending.size).toBe(0);
        expect(refresh).not.toHaveBeenCalled();

        await owner.resume();
        await flushMicrotasks();
        activeTransport = 2;
        if (transport === "sse") {
          sources[activeTransport]?.open();
          await flushMicrotasks();
          sseControls[1]?.resolve();
        } else {
          sockets[activeTransport]?.onopen?.();
          acknowledgeWebSocket(activeTransport);
        }
        await flushMicrotasks();
        expect(freshness.filter((state) => state === "current")).toHaveLength(1);
      } else if (ending === "authenticate") {
        if (transport === "sse") sseControls[0]?.resolve();
        else acknowledgeWebSocket(1);
      } else if (transport === "sse") {
        sseControls[0]?.reject(new Error("membership_rejected"));
      } else {
        sockets[1]?.onmessage?.({ data: canonicalize({ kind: "membership_rejected" }) });
      }
      await flushMicrotasks();
      if (ending === "authenticate" || ending === "suspend") {
        if (transport === "sse") sources[activeTransport]?.request.failed("transport_lost");
        else sockets[activeTransport]?.onclose?.();
      }
      await flushMicrotasks();

      const expectedFallback = scenario === "change" ? 11_000 : 33_000;
      const fallbackTimers = [...timers.pending.values()].filter(
        ({ milliseconds }) => milliseconds === expectedFallback,
      );
      if (scenario === "push-only") {
        expect(fallbackTimers).toHaveLength(0);
        expect(refresh).not.toHaveBeenCalled();
      } else {
        expect(fallbackTimers).toHaveLength(1);
        timers.fire(expectedFallback);
        expect(refresh).toHaveBeenCalledOnce();
      }
      oldFallback?.callback();
      expect(refresh).toHaveBeenCalledTimes(scenario === "push-only" ? 0 : 1);
      owner.dispose();
    },
  );

  it("falls back from a failed pageshow reauthorization without reusing the old socket", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const refresh = vi.fn(successfulFreshRender);
    const root = Object.freeze({}) as Element;
    let authorizations = 0;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () => {
            authorizations += 1;
            if (authorizations === 2) return Promise.reject(new Error("reauthorization_failed"));
            return Object.freeze({ replay: Object.freeze([]), subscription: authorization(0n) });
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-reauthorization-fallback",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    await Promise.resolve();
    await Promise.resolve();

    owner.suspend();
    expect(sources[0]?.close).toHaveBeenCalledOnce();
    await owner.resume();
    await Promise.resolve();
    await Promise.resolve();

    expect(sources).toHaveLength(1);
    expect(refresh).not.toHaveBeenCalled();
    expect(
      [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 33_000),
    ).toHaveLength(1);
    timers.fire(33_000);
    expect(refresh).toHaveBeenCalledOnce();
    owner.dispose();
  });

  it("observes push-only degradation publicly without creating fallback polling", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const refresh = vi.fn(successfulFreshRender);
    const freshness: string[] = [];
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            Object.freeze({ replay: Object.freeze([]), subscription: authorization(0n) }),
        },
        clock: { now: () => 100 },
        observeFreshness: ({ state }) => freshness.push(state),
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-push-only-freshness",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [pushOnlyOwnership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    await Promise.resolve();
    await Promise.resolve();

    expect(freshness).toEqual(["current"]);
    sources[0]?.emit(envelope(2n, { kind: "heartbeat" }));
    expect(freshness).toEqual(["current", "degraded"]);
    expect(refresh).not.toHaveBeenCalled();
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      false,
    );
    owner.dispose();
    expect(freshness[freshness.length - 1]).toBe("closed");
  });

  it.each([
    { fallback: true, mode: "hybrid" },
    { fallback: false, mode: "push-only" },
  ] as const)(
    "degrades only an exhausted $mode membership and reports resource exhaustion",
    async ({ fallback, mode }) => {
      const sources: FakeSource[] = [];
      const timers = new FakeTimers();
      const diagnose = vi.fn();
      const freshness: { documentKey: string; state: string }[] = [];
      const firstRoot = Object.freeze({}) as Element;
      const secondRoot = Object.freeze({}) as Element;
      const firstRefresh = vi.fn(
        (
          _reason: unknown,
          completion?: (outcome: "succeeded" | "failed" | "canceled" | "retired") => void,
        ) => {
          completion?.("failed");
          return "exhausted" as const;
        },
      );
      const secondRefresh = vi.fn(successfulFreshRender);
      const owner = new AsyncDocumentOwner(
        { diagnose, onDispose: vi.fn() },
        {
          authority: {
            authorize: ({ identity }) => {
              const second = identity.documentKey === "document-resource-sibling";
              return Object.freeze({
                replay: Object.freeze([]),
                subscription: authorization(0n, {
                  descriptorBinding: second ? "binding-sibling" : "binding-exhausted",
                  subscriptionId: second ? "subscription-002" : "subscription-001",
                }),
              });
            },
          },
          clock: { now: () => 100 },
          observeFreshness: ({ documentKey, state }) => freshness.push({ documentKey, state }),
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
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: firstRoot,
        enqueueFreshRender: firstRefresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: "document-resource-exhausted",
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () =>
          mode === "hybrid" ? [ownership(firstRoot)] : [pushOnlyOwnership(firstRoot)],
        writePresentationSignal: (_element, _name, value) => value,
      });
      owner.connectIsland({
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: secondRoot,
        enqueueFreshRender: secondRefresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: "document-resource-sibling",
          slot: "orders-sibling-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () => [pushOnlyOwnership(secondRoot)],
        writePresentationSignal: (_element, _name, value) => value,
      });
      await flushMicrotasks();
      sources[0]?.open();
      await flushMicrotasks();

      sources[0]?.emit(envelope(1n, { kind: "refresh", name: "refresh" }));

      expect(firstRefresh).toHaveBeenCalledOnce();
      expect(secondRefresh).not.toHaveBeenCalled();
      expect(diagnose).toHaveBeenCalledWith("resource_exhausted");
      expect(sources[0]?.unsubscribe).toHaveBeenCalledExactlyOnceWith("subscription-001");
      expect(sources[0]?.close).not.toHaveBeenCalled();
      expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
        fallback,
      );
      expect(
        freshness.filter(
          ({ documentKey, state }) =>
            documentKey === "document-resource-sibling" && state === "degraded",
        ),
      ).toHaveLength(0);

      sources[0]?.emit(
        canonicalize({
          payload: { kind: "heartbeat" },
          position: { epoch: "1", sequence: "1" },
          protocol_version: 1,
          stream: "orders",
          subscription: "subscription-002",
        }),
      );
      await flushMicrotasks();

      expect(sources).toHaveLength(1);
      expect(sources[0]?.close).not.toHaveBeenCalled();
      expect(
        freshness.filter(
          ({ documentKey, state }) =>
            documentKey === "document-resource-sibling" && state === "degraded",
        ),
      ).toHaveLength(0);
      owner.dispose();
    },
  );

  it("starts descriptor-default hybrid polling only after continuity is lost", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const refresh = vi.fn(successfulFreshRender);
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: { authorize: () => authorization(0n) },
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-hybrid-poll",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    await Promise.resolve();
    await Promise.resolve();

    sources[0]?.emit(envelope(1n, { kind: "heartbeat" }));
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      false,
    );
    sources[0]?.emit(envelope(3n, { kind: "heartbeat" }));
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      true,
    );
    timers.fire(33_000);
    expect(refresh).toHaveBeenCalledOnce();
    expect(refresh).toHaveBeenCalledWith("poll", expect.any(Function), "poll");
    owner.dispose();
  });

  it("reconciles committed stream mode changes against the current descriptor without a second transport", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const refresh = vi.fn(successfulFreshRender);
    const diagnose = vi.fn();
    const root = Object.freeze({}) as Element;
    let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([ownership(root)]);
    const owner = new AsyncDocumentOwner(
      { diagnose, onDispose: vi.fn() },
      {
        authority: { authorize: () => authorization(0n) },
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
    const controller = owner.connectIsland({
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-morph-mode",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    await Promise.resolve();
    sources[0]?.emit(envelope(3n, { kind: "heartbeat" }));
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      true,
    );

    controller.beforeMorph?.();
    ownerships = Object.freeze([pushOnlyOwnership(root)]);
    controller.afterMorph?.();
    expect(sources).toHaveLength(1);
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      false,
    );

    controller.beforeMorph?.();
    ownerships = Object.freeze([ownership(root)]);
    controller.afterMorph?.();
    expect(sources).toHaveLength(1);
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      true,
    );

    controller.beforeMorph?.();
    ownerships = Object.freeze([pushOnlyOwnership(root), pollOwnership(root, ["5s"])]);
    controller.afterMorph?.();
    expect(diagnose).toHaveBeenCalledWith("operation_rejected");
    expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 33_000)).toBe(
      false,
    );
    timers.fire(5_000);
    expect(refresh).not.toHaveBeenCalled();
    expect(sources).toHaveLength(1);
    owner.dispose();
  });

  it("restores semantic stream projection when a committed morph drops stream ownership", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const root = Object.freeze({}) as Element;
    let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([ownership(root)]);
    const clearAsyncStatus = vi.fn();
    const projectAsyncStatus = vi.fn();
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: { authorize: () => authorization(0n) },
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
    const controller = owner.connectIsland({
      clearAsyncStatus,
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: successfulFreshRender,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-projection-loss",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      projectAsyncStatus,
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: (_element, _name, value) => value,
    });
    await flushMicrotasks();
    sources[0]?.open();
    sources[0]?.emit(envelope(1n, { kind: "heartbeat" }));
    expect(projectAsyncStatus).toHaveBeenCalledWith("current");

    controller.beforeMorph?.();
    ownerships = Object.freeze([pollOwnership(root, ["30s"])]);
    controller.afterMorph?.();

    expect(clearAsyncStatus).toHaveBeenCalledOnce();
    expect(sources[0]?.close).toHaveBeenCalledOnce();
    owner.dispose();
  });

  it("does not report physical continuity for a duplicate while the logical stream is connecting", async () => {
    const sources: FakeSource[] = [];
    const continuityProved = vi.fn();
    // eslint-disable-next-line @typescript-eslint/unbound-method -- captured before the temporary prototype spy and invoked with an explicit pool receiver
    const originalSubscribe = DocumentConnectionPool.prototype.subscribe;
    const subscribe = vi
      .spyOn(DocumentConnectionPool.prototype, "subscribe")
      .mockImplementation(function (this: DocumentConnectionPool, authorized, sink, pending) {
        const handle = originalSubscribe.call(this, authorized, sink, pending);
        return Object.freeze({
          close: () => {
            handle.close();
          },
          continuityLost: () => {
            handle.continuityLost();
          },
          continuityProved: () => {
            continuityProved();
            handle.continuityProved();
          },
          heartbeatLost: () => {
            handle.heartbeatLost();
          },
          presentationFailed: () => {
            handle.presentationFailed();
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
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: successfulFreshRender,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: "document-duplicate",
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
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

  it("applies a complete initial replay only after physical membership authentication", async () => {
    const sources: FakeSource[] = [];
    const refresh = vi.fn(successfulFreshRender);
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-initial-replay",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(sources).toHaveLength(1);
    expect(refresh).not.toHaveBeenCalled();
    sources[0]?.open();
    expect(refresh).toHaveBeenCalledOnce();
    owner.dispose();
  });

  it("withholds replay continuity until its refresh reaches the real terminal outcome", async () => {
    const completions: ((completion: "succeeded" | "failed" | "canceled" | "retired") => void)[] =
      [];
    const continuityProved = vi.fn();
    const signal = vi.fn((_scope: string, _name: string, value: JsonValue) => value);
    const stages: NonNullable<Parameters<DocumentConnectionPool["subscribe"]>[2]>[] = [];
    const subscribe = vi
      .spyOn(DocumentConnectionPool.prototype, "subscribe")
      .mockImplementation((_authorization, _sink, pending) => {
        if (pending !== null && pending !== undefined) stages.push(pending);
        return Object.freeze({
          close: vi.fn(),
          continuityLost: vi.fn(),
          continuityProved,
          heartbeatLost: vi.fn(),
          presentationFailed: vi.fn(),
        });
      });
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            Object.freeze({
              replay: Object.freeze([
                envelope(1n, { kind: "refresh", name: "refresh" }),
                envelope(2n, {
                  kind: "presentation_signal",
                  name: "completion_percent",
                  scope: "root-scope",
                  value: 50,
                }),
                envelope(3n, { kind: "refresh", name: "refresh" }),
              ]),
              subscription: authorization(0n),
            }),
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: new FakeTimers().port,
        transports: {
          eventSource() {
            throw new Error("unexpected_transport");
          },
          webSocket() {
            throw new Error("unexpected_transport");
          },
        },
      },
    );
    try {
      owner.connectIsland({
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: (_reason, completion) => {
          if (completion !== undefined) completions.push(completion);
          return "queued";
        },
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: "document-pending-replay",
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: signal,
      });
      await Promise.resolve();
      await Promise.resolve();

      const staged = stages[0];
      if (staged === undefined) throw new Error("missing staged replay authorization");
      expect(staged.commit()).toBe("pending");
      expect(continuityProved).not.toHaveBeenCalled();
      expect(completions).toHaveLength(1);

      completions[0]?.("succeeded");
      expect(signal).toHaveBeenCalledExactlyOnceWith("root-scope", "completion_percent", 50);
      expect(completions).toHaveLength(2);
      expect(continuityProved).not.toHaveBeenCalled();

      completions[1]?.("succeeded");
      expect(continuityProved).toHaveBeenCalledOnce();
    } finally {
      owner.dispose();
      subscribe.mockRestore();
    }
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
    const refresh = vi.fn(successfulFreshRender);
    const event = vi.fn(() => "dispatched" as const);
    const signal = vi.fn((_scope: string, _name: string, value: JsonValue) => value);
    const port: AsyncRuntimeIslandPort = {
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: event,
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
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
      envelope(2n, {
        kind: "presentation_signal",
        name: "completion_percent",
        scope: "root-scope",
        value: 75,
      }),
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
    expect(signal).toHaveBeenCalledWith("root-scope", "completion_percent", 75);
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
    expect(refresh).toHaveBeenCalledOnce();
    sources[1]?.open();
    expect(refresh).toHaveBeenCalledTimes(2);
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    } satisfies AsyncRuntimeIslandPort;
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
    const consumeRegisteredEventCapability = vi.fn<
      AsyncRuntimeIslandPort["consumeRegisteredEventCapability"]
    >(() => {
      const capability = Object.freeze({});
      capabilities.push(capability);
      return capability as ReturnType<AsyncRuntimeIslandPort["consumeRegisteredEventCapability"]>;
    });
    const dispatch = vi.fn<AsyncRuntimeIslandPort["dispatchRegisteredEvent"]>(
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
      consumeRegisteredEventCapability,
      dispatchRegisteredEvent: dispatch,
      element: root,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-rotation",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
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

    expect(consumeRegisteredEventCapability).toHaveBeenCalledTimes(2);
    expect(Object.keys(consumeRegisteredEventCapability.mock.calls[1]?.[0] ?? {})).toEqual([]);
    expect(dispatch.mock.calls[0]?.[0]).toBe(capabilities[1]);
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(777);
    owner.dispose();
  });

  it("uses the staged successor heartbeat while ordinary reconnect authentication is pending", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const rotated = authorization(0n, {
      descriptorBinding: "binding-pending-heartbeat",
      heartbeatTimeoutMs: 777,
    });
    let calls = 0;
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize() {
            calls += 1;
            return calls === 1
              ? authorization(0n)
              : Object.freeze({ replay: Object.freeze([]), subscription: rotated });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: {
          eventSource(request) {
            const source = new FakeSource(request);
            if (sources.length === 1) {
              source.subscribe.mockImplementation(() => new Promise(() => undefined));
            }
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-pending-heartbeat",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();
    sources[0]?.request.failed("transport_lost");
    timers.fire(50);
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();

    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(777);
    expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).not.toContain(
      5_000,
    );
    owner.dispose();
  });

  it("stages an ordinary reconnect tail until the replacement membership authenticates", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const requests: AsyncAuthorizationRequest[] = [];
    const refresh = vi.fn(successfulFreshRender);
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-reconnect",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
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
    expect(refresh).not.toHaveBeenCalled();
    expect(sources).toHaveLength(2);
    sources[1]?.open();
    expect(refresh).toHaveBeenCalledOnce();
    sources[1]?.emit(envelope(3n, { kind: "refresh", name: "refresh" }));
    expect(refresh).toHaveBeenCalledTimes(2);
    owner.dispose();
  });

  it("coalesces the first gap into one immediate reconnect from the last committed position", async () => {
    const sources: FakeSource[] = [];
    const timers = new FakeTimers();
    const requests: AsyncAuthorizationRequest[] = [];
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
              : Object.freeze({ replay: Object.freeze([]), subscription: current });
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: successfulFreshRender,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-gap",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();

    sources[0]?.emit(envelope(3n, { kind: "heartbeat" }));
    sources[0]?.emit(envelope(4n, { kind: "heartbeat" }));
    expect(sources[0]?.close).toHaveBeenCalledOnce();
    expect(requests).toHaveLength(1);
    timers.fire(50);
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();

    expect(requests).toHaveLength(2);
    expect(requests[1]?.position).toEqual({ epoch: 1n, sequence: 0n });
    sources[0]?.emit(envelope(5n, { kind: "heartbeat" }));
    expect(requests).toHaveLength(2);
    owner.dispose();
  });

  it("keeps initial replay inert through the real SSE adapter until its exact control acknowledgment", async () => {
    const timers = new FakeTimers();
    const native: {
      close: ReturnType<typeof vi.fn>;
      onerror?: VoidFunction;
      onopen?: VoidFunction;
    }[] = [];
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
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
        return new Promise((resolve) => controls.push({ request, resolve }));
      },
      timers: timers.port,
      webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
    });
    const refresh = vi.fn(successfulFreshRender);
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            const position = request.position?.sequence ?? 0n;
            return Object.freeze({
              replay: Object.freeze([
                envelope(position + 1n, { kind: "refresh", name: "refresh" }),
              ]),
              subscription: authorization(position),
            });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      },
    );
    owner.connectIsland({
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-real-sse",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    native[0]?.onopen?.();
    expect(refresh).not.toHaveBeenCalled();
    const control = controls[0];
    if (control === undefined) throw new Error("missing_control");
    control.resolve({
      connection: control.request.connection,
      controlNonce: control.request.controlNonce,
      descriptorBinding: control.request.subscription.descriptorBinding,
      kind: "authenticated",
      operation: control.request.operation,
      stream: control.request.subscription.stream,
      subscriptionId: control.request.subscription.subscriptionId,
      transportGeneration: control.request.transportGeneration,
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(refresh).toHaveBeenCalledOnce();

    native[0]?.onerror?.();
    timers.fire(50);
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
    native[1]?.onopen?.();
    expect(refresh).toHaveBeenCalledOnce();
    const reconnectControl = controls[1];
    if (reconnectControl === undefined) throw new Error("missing_reconnect_control");
    reconnectControl.resolve({
      connection: reconnectControl.request.connection,
      controlNonce: reconnectControl.request.controlNonce,
      descriptorBinding: reconnectControl.request.subscription.descriptorBinding,
      kind: "authenticated",
      operation: reconnectControl.request.operation,
      stream: reconnectControl.request.subscription.stream,
      subscriptionId: reconnectControl.request.subscription.subscriptionId,
      transportGeneration: reconnectControl.request.transportGeneration,
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(refresh).toHaveBeenCalledTimes(2);
    owner.dispose();
  });

  it("keeps initial replay inert through the real WebSocket adapter until its exact ACK", async () => {
    const timers = new FakeTimers();
    const sent: string[] = [];
    const sockets: {
      close: ReturnType<typeof vi.fn>;
      onmessage?: (event?: unknown) => void;
      onopen?: VoidFunction;
      send(data: string): void;
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: timers.port,
      webSocket() {
        const socket = {
          close: vi.fn(),
          send(data: string) {
            sent.push(data);
          },
        };
        sockets.push(socket);
        return socket;
      },
    });
    const refresh = vi.fn(successfulFreshRender);
    const root = Object.freeze({}) as Element;
    const websocketAuthorization = authorization(0n, {
      document: Object.freeze({
        authorizationScope: "document-scope",
        origin: "https://app.example.test",
        transport: "websocket" as const,
      }),
    });
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize: () =>
            Object.freeze({
              replay: Object.freeze([envelope(1n, { kind: "refresh", name: "refresh" })]),
              subscription: websocketAuthorization,
            }),
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      },
    );
    owner.connectIsland({
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-real-websocket",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sockets[0]?.onopen?.();
    expect(refresh).not.toHaveBeenCalled();
    const request = JSON.parse(sent[0] ?? "null") as Record<string, unknown>;
    sockets[0]?.onmessage?.({
      data: canonicalize({
        control_nonce: String(request["control_nonce"]),
        descriptor_binding: String(request["descriptor_binding"]),
        kind: "membership_authenticated",
        stream: String(request["stream"]),
        subscription: String(request["subscription"]),
        transport_generation: Number(request["transport_generation"]),
      }),
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(refresh).toHaveBeenCalledOnce();
    owner.dispose();
  });

  it.each([
    { lifecycle: "ordinary reconnect", replay: false },
    { lifecycle: "ordinary reconnect", replay: true },
    { lifecycle: "bfcache resume", replay: false },
    { lifecycle: "bfcache resume", replay: true },
  ])(
    "resets retry authority after $lifecycle with $replay replay entries and no live successor",
    async ({ lifecycle, replay }) => {
      const sources: FakeSource[] = [];
      const timers = new FakeTimers();
      const root = Object.freeze({}) as Element;
      const reconnect = Object.freeze({
        kind: "resume_or_refresh" as const,
        maximumAttempts: 1,
        maximumDelayMs: 100,
        minimumDelayMs: 100,
      });
      const owner = new AsyncDocumentOwner(
        { diagnose: vi.fn(), onDispose: vi.fn() },
        {
          authority: {
            authorize(request) {
              const position = request.position?.sequence ?? 0n;
              const current = authorization(position, { reconnect });
              if (request.prior === null) return current;
              return Object.freeze({
                replay: replay
                  ? Object.freeze([envelope(position + 1n, { kind: "refresh", name: "refresh" })])
                  : Object.freeze([]),
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
        consumeRegisteredEventCapability: eventCapability,
        dispatchRegisteredEvent: () => "dispatched",
        element: root,
        enqueueFreshRender: successfulFreshRender,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `document-silent-${lifecycle}-${String(replay)}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: (_element, _name, value) => value,
      });
      await Promise.resolve();
      await Promise.resolve();
      sources[0]?.open();

      if (lifecycle === "bfcache resume") {
        owner.suspend();
        await owner.resume();
        sources[1]?.open();
      }

      const firstRecoverySource = sources[sources.length - 1];
      firstRecoverySource?.request.failed("transport_lost");
      timers.fire(50);
      for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
      const replacement = sources[sources.length - 1];
      expect(replacement).not.toBe(firstRecoverySource);
      replacement?.open();
      replacement?.request.failed("transport_lost");

      expect([...timers.pending.values()].map(({ milliseconds }) => milliseconds)).toContain(50);
      owner.dispose();
    },
  );

  it("bounds repeated noncooperative bfcache authority and ignores every late settlement", async () => {
    const timers = new FakeTimers();
    const sources: FakeSource[] = [];
    const diagnose = vi.fn();
    const late: ((value: AuthorizedLogicalSubscription) => void)[] = [];
    const signals: AbortSignal[] = [];
    let calls = 0;
    const root = Object.freeze({}) as Element;
    const owner = new AsyncDocumentOwner(
      { diagnose, onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            calls += 1;
            if (calls === 1) return authorization(0n);
            signals.push(request.signal);
            return new Promise<AuthorizedLogicalSubscription>((resolve) => {
              late.push(resolve);
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: () => "dispatched",
      element: root,
      enqueueFreshRender: () => "queued",
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-noncooperative-authority",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: (_element, _name, value) => value,
    });
    await Promise.resolve();
    await Promise.resolve();
    sources[0]?.open();

    for (let attempt = 0; attempt < 3; attempt += 1) {
      owner.suspend();
      const resumed = owner.resume();
      await Promise.resolve();
      expect(
        [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 5_000),
      ).toHaveLength(2);
      timers.fire(5_000);
      await resumed;
      await Promise.resolve();
      expect(signals[attempt]?.aborted).toBe(true);
      expect(
        [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 33_000),
      ).toHaveLength(1);
    }

    expect(late).toHaveLength(3);
    late[0]?.(authorization(0n));
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
    expect(sources).toHaveLength(1);
    expect(
      [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 33_000),
    ).toHaveLength(1);
    expect(diagnose).not.toHaveBeenCalled();

    owner.dispose();
    for (const resolve of late.slice(1)) resolve(authorization(0n));
    for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();

    expect(sources).toHaveLength(1);
    expect(timers.pending.size).toBe(0);
    expect(diagnose).not.toHaveBeenCalled();
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
      consumeRegisteredEventCapability: eventCapability,
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "document-001",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    } satisfies AsyncRuntimeIslandPort;
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
