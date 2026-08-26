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
  for (let turn = 0; turn < 8; turn += 1) await Promise.resolve();
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
});
