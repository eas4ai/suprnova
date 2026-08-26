import { describe, expect, it, vi } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import {
  AsyncDocumentOwner,
  type AsyncAuthorizationRequest,
} from "../src/async-updates/feature.js";
import {
  BrowserAsyncTransportPorts,
  type BrowserAsyncTransportOptions,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  RuntimeFeatureDirectiveOwnership,
  RuntimeFeatureIslandPort,
} from "../src/features/contract.js";

type Transport = "sse" | "websocket";

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

  fireAll(milliseconds: number): void {
    const ready = [...this.pending].filter(([, timer]) => timer.milliseconds === milliseconds);
    if (ready.length === 0) throw new Error(`timer_not_found:${String(milliseconds)}`);
    for (const [handle, timer] of ready) {
      this.pending.delete(handle);
      timer.callback();
    }
  }

  fireOne(milliseconds: number): void {
    const ready = [...this.pending].find(([, timer]) => timer.milliseconds === milliseconds);
    if (ready === undefined) throw new Error(`timer_not_found:${String(milliseconds)}`);
    this.pending.delete(ready[0]);
    ready[1].callback();
  }
}

function authorization(call: number, transport: Transport): AuthorizedLogicalSubscription {
  const baseline = call === 1 ? 0n : 10n;
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: baseline }),
    descriptorBinding: `binding-${String(call)}`,
    document: Object.freeze({
      authorizationScope: `document-scope-${String(call)}`,
      origin: `https://app-${String(call)}.example.test`,
      transport,
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
      maximumDelayMs: 400,
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

function eventCapability(
  request?: Parameters<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]>[0],
): ReturnType<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]> {
  void request;
  return Object.freeze({}) as ReturnType<RuntimeFeatureIslandPort["authorizeRegisteredEvents"]>;
}

async function settle(): Promise<void> {
  for (let turn = 0; turn < 32; turn += 1) await Promise.resolve();
}

function websocketAck(socket: { readonly sent: string[] }): string {
  const request = JSON.parse(socket.sent[0] ?? "null") as Record<string, unknown>;
  return canonicalize({
    control_nonce: String(request["control_nonce"]),
    descriptor_binding: String(request["descriptor_binding"]),
    kind: "membership_authenticated",
    stream: String(request["stream"]),
    subscription: String(request["subscription"]),
    transport_generation: Number(request["transport_generation"]),
  });
}

describe("pre-authentication bfcache continuity", () => {
  it.each(
    (["sse", "websocket"] as const).flatMap((transport) =>
      (["raw", "replay", "no_tail"] as const).map((evidence) => ({ evidence, transport })),
    ),
  )(
    "reacquires fresh initial $evidence authority for real $transport after persisted suspension",
    async ({ evidence, transport }) => {
      const timers = new Timers();
      const controls: {
        request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
        resolve(value: unknown): void;
      }[] = [];
      const sources: {
        close: ReturnType<typeof vi.fn>;
        onmessage?: (event: Readonly<{ data: string }>) => void;
        onopen?: VoidFunction;
      }[] = [];
      const sockets: {
        close: ReturnType<typeof vi.fn>;
        onmessage?: (event: Readonly<{ data: string }>) => void;
        onopen?: VoidFunction;
        send(data: string): void;
        readonly sent: string[];
      }[] = [];
      const transports = new BrowserAsyncTransportPorts({
        eventSource() {
          const source = { close: vi.fn() };
          sources.push(source);
          return source;
        },
        fetch: vi.fn<typeof globalThis.fetch>(),
        membershipTimeoutMs: 5_000,
        sseMembership(request) {
          return new Promise((resolve) => controls.push({ request, resolve }));
        },
        timers: timers.port,
        webSocket() {
          const sent: string[] = [];
          const socket = { close: vi.fn(), send: (data: string) => sent.push(data), sent };
          sockets.push(socket);
          return socket;
        },
      });
      const requests: AsyncAuthorizationRequest[] = [];
      const refresh = vi.fn(() => "queued" as const);
      const authorizeEvents = vi.fn(eventCapability);
      const signal = vi.fn((_element: Element, _name: string, value: JsonValue) => value);
      let calls = 0;
      const owner = new AsyncDocumentOwner(
        { diagnose: vi.fn(), onDispose: vi.fn() },
        {
          authority: {
            authorize(request) {
              requests.push(request);
              calls += 1;
              const current = authorization(calls, transport);
              if (evidence === "raw") return current;
              return Object.freeze({
                replay:
                  evidence === "replay"
                    ? Object.freeze([
                        envelope(current.baseline.sequence + 1n, {
                          kind: "refresh",
                          name: "refresh",
                        }),
                      ])
                    : Object.freeze([]),
                subscription: current,
              });
            },
          },
          clock: { now: () => 100 },
          randomness: { number: () => 0.5 },
          timers: timers.port,
          transports,
        },
      );
      const root = Object.freeze({}) as Element;
      owner.connectIsland({
        authorizeRegisteredEvents: authorizeEvents,
        dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
        element: root,
        enqueueFreshRender: refresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `preack-${transport}-${evidence}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        proposeUploadHandle: vi.fn(() => "accepted" as const),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: signal,
      });
      await settle();

      if (transport === "sse") sources[0]?.onopen?.();
      else sockets[0]?.onopen?.();
      await settle();
      expect(authorizeEvents).not.toHaveBeenCalled();
      expect(refresh).not.toHaveBeenCalled();

      owner.suspend();
      await owner.resume();
      await settle();

      expect(requests).toHaveLength(2);
      expect(
        requests.map(({ position, prior }) => ({
          position: position === null ? null : position.sequence,
          prior: prior?.subscriptionId ?? null,
        })),
      ).toEqual([
        { position: null, prior: null },
        { position: null, prior: null },
      ]);
      expect(transport === "sse" ? sources : sockets).toHaveLength(2);
      if (transport === "sse") sources[1]?.onopen?.();
      else sockets[1]?.onopen?.();
      await settle();

      if (transport === "sse") {
        const old = controls[0];
        if (old === undefined) throw new Error("missing_old_sse_control");
        old.resolve({
          connection: old.request.connection,
          controlNonce: old.request.controlNonce,
          descriptorBinding: old.request.subscription.descriptorBinding,
          kind: "authenticated",
          operation: old.request.operation,
          stream: old.request.subscription.stream,
          subscriptionId: old.request.subscription.subscriptionId,
          transportGeneration: old.request.transportGeneration,
        });
        sources[0]?.onmessage?.({
          data: envelope(1n, { kind: "refresh", name: "refresh" }),
        });
      } else {
        const old = sockets[0];
        if (old === undefined) throw new Error("missing_old_websocket");
        old.onmessage?.({ data: websocketAck(old) });
        old.onmessage?.({ data: envelope(1n, { kind: "refresh", name: "refresh" }) });
      }
      await settle();
      expect(authorizeEvents).not.toHaveBeenCalled();
      expect(refresh).not.toHaveBeenCalled();
      expect(signal).not.toHaveBeenCalled();

      if (transport === "sse") {
        const fresh = controls[1];
        if (fresh === undefined) throw new Error("missing_fresh_sse_control");
        fresh.resolve({
          connection: fresh.request.connection,
          controlNonce: fresh.request.controlNonce,
          descriptorBinding: fresh.request.subscription.descriptorBinding,
          kind: "authenticated",
          operation: fresh.request.operation,
          stream: fresh.request.subscription.stream,
          subscriptionId: fresh.request.subscription.subscriptionId,
          transportGeneration: fresh.request.transportGeneration,
        });
      } else {
        const fresh = sockets[1];
        if (fresh === undefined) throw new Error("missing_fresh_websocket");
        fresh.onmessage?.({ data: websocketAck(fresh) });
      }
      await settle();

      expect(authorizeEvents).toHaveBeenCalledOnce();
      expect(authorizeEvents.mock.calls[0]?.[0]?.descriptorBinding).toBe("binding-2");
      expect(refresh).toHaveBeenCalledTimes(evidence === "replay" ? 1 : 0);
      expect(signal).not.toHaveBeenCalled();
      expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 5_000)).toBe(
        true,
      );
      owner.dispose();
      expect(timers.pending.size).toBe(0);
    },
  );

  it("bounds repeated noncooperative fresh-initial authority and fences every late result", async () => {
    const timers = new Timers();
    const sources: {
      close: ReturnType<typeof vi.fn>;
      onopen?: VoidFunction;
    }[] = [];
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource() {
        const source = { close: vi.fn() };
        sources.push(source);
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
    const requests: AsyncAuthorizationRequest[] = [];
    const late: ((authorization: AuthorizedLogicalSubscription) => void)[] = [];
    const signals: AbortSignal[] = [];
    let calls = 0;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            requests.push(request);
            calls += 1;
            if (calls === 1) return authorization(1, "sse");
            signals.push(request.signal);
            return new Promise<AuthorizedLogicalSubscription>((resolve) => late.push(resolve));
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      },
    );
    const root = Object.freeze({}) as Element;
    owner.connectIsland({
      authorizeRegisteredEvents: eventCapability,
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "preack-noncooperative",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: vi.fn(() => "accepted" as const),
      queryDirectiveOwnership: () => [ownership(root)],
      writePresentationSignal: vi.fn((_element: Element, _name: string, value: JsonValue) => value),
    });
    await settle();
    sources[0]?.onopen?.();
    await settle();
    expect(controls).toHaveLength(1);

    for (let attempt = 0; attempt < 3; attempt += 1) {
      owner.suspend();
      const resumed = owner.resume();
      await settle();
      expect(signals[attempt]?.aborted).toBe(false);
      timers.fireAll(5_000);
      await resumed;
      await settle();
      expect(signals[attempt]?.aborted).toBe(true);
      expect(timers.pending.size).toBe(0);
      expect(sources).toHaveLength(1);
    }

    expect(requests).toHaveLength(4);
    expect(
      requests.map(({ position, prior }) => ({
        position: position === null ? null : position.sequence,
        prior: prior?.subscriptionId ?? null,
      })),
    ).toEqual([
      { position: null, prior: null },
      { position: null, prior: null },
      { position: null, prior: null },
      { position: null, prior: null },
    ]);
    for (const [index, resolve] of late.entries()) resolve(authorization(index + 2, "sse"));
    await settle();
    expect(sources).toHaveLength(1);
    expect(timers.pending.size).toBe(0);

    owner.dispose();
    expect(timers.pending.size).toBe(0);
  });

  it.each(
    (["sse", "websocket"] as const).flatMap((transport) =>
      (["raw", "replay", "no_tail"] as const).flatMap((evidence) =>
        (["unresolved", "rejected", "timed_out"] as const).map((ending) => ({
          ending,
          evidence,
          transport,
        })),
      ),
    ),
  )(
    "restarts $transport fresh-initial $evidence authority after an orphaned $ending first request",
    async ({ ending, evidence, transport }) => {
      const timers = new Timers();
      const controls: {
        request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
        resolve(value: unknown): void;
      }[] = [];
      const sources: {
        close: ReturnType<typeof vi.fn>;
        onmessage?: (event: Readonly<{ data: string }>) => void;
        onopen?: VoidFunction;
      }[] = [];
      const sockets: {
        close: ReturnType<typeof vi.fn>;
        onmessage?: (event: Readonly<{ data: string }>) => void;
        onopen?: VoidFunction;
        send(data: string): void;
        readonly sent: string[];
      }[] = [];
      const transports = new BrowserAsyncTransportPorts({
        eventSource() {
          const source = { close: vi.fn() };
          sources.push(source);
          return source;
        },
        fetch: vi.fn<typeof globalThis.fetch>(),
        membershipTimeoutMs: 5_000,
        sseMembership(request) {
          return new Promise((resolve) => controls.push({ request, resolve }));
        },
        timers: timers.port,
        webSocket() {
          const sent: string[] = [];
          const socket = { close: vi.fn(), send: (data: string) => sent.push(data), sent };
          sockets.push(socket);
          return socket;
        },
      });
      const requests: AsyncAuthorizationRequest[] = [];
      let resolveOld: ((value: AuthorizedLogicalSubscription) => void) | undefined;
      let rejectOld: ((reason?: unknown) => void) | undefined;
      let calls = 0;
      const owner = new AsyncDocumentOwner(
        { diagnose: vi.fn(), onDispose: vi.fn() },
        {
          authority: {
            authorize(request) {
              requests.push(request);
              calls += 1;
              if (calls === 1) {
                return new Promise<AuthorizedLogicalSubscription>((resolve, reject) => {
                  resolveOld = resolve;
                  rejectOld = reject;
                });
              }
              const current = authorization(2, transport);
              if (evidence === "raw") return current;
              return Object.freeze({
                replay:
                  evidence === "replay"
                    ? Object.freeze([
                        envelope(current.baseline.sequence + 1n, {
                          kind: "refresh",
                          name: "refresh",
                        }),
                      ])
                    : Object.freeze([]),
                subscription: current,
              });
            },
          },
          clock: { now: () => 100 },
          randomness: { number: () => 0.5 },
          timers: timers.port,
          transports,
        },
      );
      const authorizeEvents = vi.fn(eventCapability);
      const refresh = vi.fn(() => "queued" as const);
      const signal = vi.fn((_element: Element, _name: string, value: JsonValue) => value);
      const root = Object.freeze({}) as Element;
      owner.connectIsland({
        authorizeRegisteredEvents: authorizeEvents,
        dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
        element: root,
        enqueueFreshRender: refresh,
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `preinstall-${transport}-${evidence}-${ending}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        proposeUploadHandle: vi.fn(() => "accepted" as const),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: signal,
      });
      await settle();
      expect(requests).toHaveLength(1);
      expect(sources).toHaveLength(0);
      expect(sockets).toHaveLength(0);

      if (ending === "timed_out") {
        timers.fireAll(5_000);
        await settle();
      }
      owner.suspend();
      await settle();
      if (ending === "rejected") rejectOld?.(new Error("late_initial_rejection"));
      const resumed = owner.resume();
      await resumed;
      await settle();

      expect(requests).toHaveLength(2);
      expect(
        requests.map(({ position, prior }) => ({
          position: position === null ? null : position.sequence,
          prior: prior?.subscriptionId ?? null,
        })),
      ).toEqual([
        { position: null, prior: null },
        { position: null, prior: null },
      ]);
      expect(transport === "sse" ? sources : sockets).toHaveLength(1);
      if (transport === "sse") sources[0]?.onopen?.();
      else sockets[0]?.onopen?.();
      await settle();

      if (ending !== "rejected") resolveOld?.(authorization(1, transport));
      await settle();
      expect(transport === "sse" ? sources : sockets).toHaveLength(1);
      expect(authorizeEvents).not.toHaveBeenCalled();
      expect(refresh).not.toHaveBeenCalled();
      expect(signal).not.toHaveBeenCalled();

      if (transport === "sse") {
        const fresh = controls[0];
        if (fresh === undefined) throw new Error("missing_orphan_recovery_sse_control");
        fresh.resolve({
          connection: fresh.request.connection,
          controlNonce: fresh.request.controlNonce,
          descriptorBinding: fresh.request.subscription.descriptorBinding,
          kind: "authenticated",
          operation: fresh.request.operation,
          stream: fresh.request.subscription.stream,
          subscriptionId: fresh.request.subscription.subscriptionId,
          transportGeneration: fresh.request.transportGeneration,
        });
      } else {
        const fresh = sockets[0];
        if (fresh === undefined) throw new Error("missing_orphan_recovery_websocket");
        fresh.onmessage?.({ data: websocketAck(fresh) });
      }
      await settle();

      expect(authorizeEvents).toHaveBeenCalledOnce();
      expect(authorizeEvents.mock.calls[0]?.[0]?.descriptorBinding).toBe("binding-2");
      expect(refresh).toHaveBeenCalledTimes(evidence === "replay" ? 1 : 0);
      expect(signal).not.toHaveBeenCalled();
      expect([...timers.pending.values()].some(({ milliseconds }) => milliseconds === 5_000)).toBe(
        true,
      );
      owner.dispose();
      expect(timers.pending.size).toBe(0);
    },
  );

  it("bounds orphaned pre-install resume authority and cancels queued work across lifecycles", async () => {
    const timers = new Timers();
    const sources: { close: ReturnType<typeof vi.fn> }[] = [];
    const pending: {
      request: AsyncAuthorizationRequest;
      resolve(value: AuthorizedLogicalSubscription): void;
    }[] = [];
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            return new Promise<AuthorizedLogicalSubscription>((resolve) => {
              pending.push({ request, resolve });
            });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports: new BrowserAsyncTransportPorts({
          eventSource() {
            const source = { close: vi.fn() };
            sources.push(source);
            return source;
          },
          fetch: vi.fn<typeof globalThis.fetch>(),
          membershipTimeoutMs: 5_000,
          sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
          timers: timers.port,
          webSocket: vi.fn<BrowserAsyncTransportOptions["webSocket"]>(),
        }),
      },
    );
    for (let island = 0; island < 9; island += 1) {
      const root = Object.freeze({}) as Element;
      owner.connectIsland({
        authorizeRegisteredEvents: eventCapability,
        dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
        element: root,
        enqueueFreshRender: vi.fn(() => "queued" as const),
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `orphan-bounded-${String(island)}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        proposeUploadHandle: vi.fn(() => "accepted" as const),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: vi.fn(
          (_element: Element, _name: string, value: JsonValue) => value,
        ),
      });
    }
    await settle();
    expect(pending).toHaveLength(8);

    owner.suspend();
    expect(pending.every(({ request }) => request.signal.aborted)).toBe(true);
    const firstResume = owner.resume();
    await settle();
    expect(pending).toHaveLength(16);
    expect(pending.slice(8).every(({ request }) => !request.signal.aborted)).toBe(true);

    const released = pending[8];
    if (released === undefined) throw new Error("missing_bounded_initial_resume");
    const releasedAuthorization = authorization(2, "sse");
    released.resolve(
      Object.freeze({
        ...releasedAuthorization,
        descriptorBinding: "binding-bounded-release",
        subscriptionId: "subscription-bounded-release",
      }),
    );
    await settle();
    expect(pending).toHaveLength(17);
    expect(sources).toHaveLength(1);

    owner.suspend();
    await firstResume;
    expect(released.request.signal.aborted).toBe(false);
    expect(pending.slice(9).every(({ request }) => request.signal.aborted)).toBe(true);
    const secondResume = owner.resume();
    await settle();
    expect(pending).toHaveLength(25);
    expect(pending.slice(17).every(({ request }) => !request.signal.aborted)).toBe(true);

    owner.dispose();
    await secondResume;
    expect(pending.slice(17).every(({ request }) => request.signal.aborted)).toBe(true);
    for (const [index, request] of pending.entries()) {
      const late = authorization(2, "sse");
      request.resolve(
        Object.freeze({
          ...late,
          descriptorBinding: `binding-late-${String(index)}`,
          subscriptionId: `subscription-late-${String(index)}`,
        }),
      );
    }
    await settle();
    expect(sources).toHaveLength(1);
    expect(timers.pending.size).toBe(0);
  });

  it("admits committed recovery fairly before its deadline behind eight orphan resumes", async () => {
    const timers = new Timers();
    const sources: {
      close: ReturnType<typeof vi.fn>;
      onopen?: VoidFunction;
    }[] = [];
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource() {
        const source = { close: vi.fn() };
        sources.push(source);
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
    const lateOrphans: ((value: AuthorizedLogicalSubscription) => void)[] = [];
    const orphanSignals: AbortSignal[] = [];
    let active = 0;
    let maximumActive = 0;
    let poolCalls = 0;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        authority: {
          authorize(request) {
            if (request.identity.documentKey === "fair-pool") {
              active += 1;
              maximumActive = Math.max(maximumActive, active);
              poolCalls += 1;
              const current = Object.freeze({
                ...authorization(poolCalls, "sse"),
                baseline: authorization(1, "sse").baseline,
                document: authorization(1, "sse").document,
              });
              const result =
                poolCalls === 1
                  ? current
                  : Object.freeze({ replay: Object.freeze([]), subscription: current });
              active -= 1;
              return result;
            }
            active += 1;
            maximumActive = Math.max(maximumActive, active);
            orphanSignals.push(request.signal);
            let activeCall = true;
            request.signal.addEventListener(
              "abort",
              () => {
                if (!activeCall) return;
                activeCall = false;
                active -= 1;
              },
              { once: true },
            );
            return new Promise<AuthorizedLogicalSubscription>((resolve) => {
              lateOrphans.push((value) => {
                if (activeCall) {
                  activeCall = false;
                  active -= 1;
                }
                resolve(value);
              });
            });
          },
        },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: timers.port,
        transports,
      },
    );
    const poolRoot = Object.freeze({}) as Element;
    const authorizeEvents = vi.fn(eventCapability);
    owner.connectIsland({
      authorizeRegisteredEvents: authorizeEvents,
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: poolRoot,
      enqueueFreshRender: vi.fn(() => "queued" as const),
      identity: Object.freeze({
        component: "fixture.orders",
        documentKey: "fair-pool",
        slot: "orders-slot",
      }),
      onDispose: vi.fn(),
      proposeUploadHandle: vi.fn(() => "accepted" as const),
      queryDirectiveOwnership: () => [ownership(poolRoot)],
      writePresentationSignal: vi.fn((_element: Element, _name: string, value: JsonValue) => value),
    });
    await settle();
    sources[0]?.onopen?.();
    await settle();
    const initialControl = controls[0];
    if (initialControl === undefined) throw new Error("missing_fair_initial_control");
    initialControl.resolve({
      connection: initialControl.request.connection,
      controlNonce: initialControl.request.controlNonce,
      descriptorBinding: initialControl.request.subscription.descriptorBinding,
      kind: "authenticated",
      operation: initialControl.request.operation,
      stream: initialControl.request.subscription.stream,
      subscriptionId: initialControl.request.subscription.subscriptionId,
      transportGeneration: initialControl.request.transportGeneration,
    });
    await settle();
    expect(authorizeEvents).toHaveBeenCalledOnce();

    for (let island = 0; island < 9; island += 1) {
      const root = Object.freeze({}) as Element;
      owner.connectIsland({
        authorizeRegisteredEvents: eventCapability,
        dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
        element: root,
        enqueueFreshRender: vi.fn(() => "queued" as const),
        identity: Object.freeze({
          component: "fixture.orders",
          documentKey: `fair-orphan-${String(island)}`,
          slot: "orders-slot",
        }),
        onDispose: vi.fn(),
        proposeUploadHandle: vi.fn(() => "accepted" as const),
        queryDirectiveOwnership: () => [ownership(root)],
        writePresentationSignal: vi.fn(
          (_element: Element, _name: string, value: JsonValue) => value,
        ),
      });
    }
    await settle();
    expect(active).toBe(8);
    expect(maximumActive).toBe(8);

    owner.suspend();
    expect(active).toBe(0);
    await settle();
    const resumed = owner.resume();
    await settle();
    expect(active).toBe(8);
    expect(poolCalls).toBe(1);
    expect(
      [...timers.pending.values()].filter(({ milliseconds }) => milliseconds === 5_000),
    ).toHaveLength(8);

    timers.fireOne(5_000);
    await settle();
    await settle();

    expect(poolCalls).toBe(2);
    expect(maximumActive).toBe(8);
    expect(sources).toHaveLength(2);
    sources[1]?.onopen?.();
    await settle();
    const restoredControl = controls[1];
    if (restoredControl === undefined) throw new Error("missing_fair_restored_control");
    restoredControl.resolve({
      connection: restoredControl.request.connection,
      controlNonce: restoredControl.request.controlNonce,
      descriptorBinding: restoredControl.request.subscription.descriptorBinding,
      kind: "authenticated",
      operation: restoredControl.request.operation,
      stream: restoredControl.request.subscription.stream,
      subscriptionId: restoredControl.request.subscription.subscriptionId,
      transportGeneration: restoredControl.request.transportGeneration,
    });
    await settle();
    expect(authorizeEvents).toHaveBeenCalledTimes(2);
    expect(authorizeEvents.mock.calls[1]?.[0]?.descriptorBinding).toBe("binding-2");

    owner.suspend();
    await resumed;
    expect(active).toBe(0);
    expect(orphanSignals.every((signal) => signal.aborted)).toBe(true);
    for (const [index, resolve] of lateOrphans.entries()) {
      const late = authorization(2, "sse");
      resolve(
        Object.freeze({
          ...late,
          descriptorBinding: `binding-fair-late-${String(index)}`,
          subscriptionId: `subscription-fair-late-${String(index)}`,
        }),
      );
    }
    await settle();
    expect(sources).toHaveLength(2);
    owner.dispose();
    expect(timers.pending.size).toBe(0);
  });
});
