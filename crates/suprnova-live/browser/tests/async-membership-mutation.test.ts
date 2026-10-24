import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import {
  BrowserAsyncTransportPorts,
  DocumentConnectionPool,
  OriginHandshakeScheduler,
  type BrowserAsyncTransportOptions,
  type SseMembershipAcknowledgment,
} from "../src/async-updates/connections.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import { eventLoopBarrier } from "./support/event-loop-barrier.js";

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
    const found = [...this.pending].find(([, timer]) => timer.milliseconds === milliseconds);
    if (found === undefined) throw new Error(`timer_not_found:${String(milliseconds)}`);
    this.pending.delete(found[0]);
    found[1].callback();
  }
}

function authorization(transport: "sse" | "websocket"): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: "binding-current",
    document: Object.freeze({
      authorizationScope: "document-scope",
      origin: "https://app.example.test",
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

function sseAcknowledgment(
  request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0],
): SseMembershipAcknowledgment {
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

interface MutationModel {
  authorization: object;
  capability: object;
  position: bigint;
  readonly effects: {
    event(): void;
    refresh(): void;
    render(): void;
    signal(): void;
  };
}

function model(): MutationModel {
  return {
    authorization: Object.freeze({ generation: "committed" }),
    capability: Object.freeze({ generation: "committed" }),
    effects: {
      event: vi.fn(),
      refresh: vi.fn(),
      render: vi.fn(),
      signal: vi.fn(),
    },
    position: 0n,
  };
}

describe("real adapter membership mutation boundary", () => {
  it.each([
    { ending: "rejected", transport: "sse" as const },
    { ending: "timeout", transport: "sse" as const },
    { ending: "transport_loss", transport: "sse" as const },
    { ending: "late", transport: "sse" as const },
    { ending: "foreign", transport: "sse" as const },
    { ending: "rejected", transport: "websocket" as const },
    { ending: "timeout", transport: "websocket" as const },
    { ending: "transport_loss", transport: "websocket" as const },
    { ending: "late", transport: "websocket" as const },
    { ending: "foreign", transport: "websocket" as const },
  ])("keeps staged state inert for $transport $ending", async ({ ending, transport }) => {
    const timers = new Timers();
    const controls: {
      reject(reason?: unknown): void;
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
    }[] = [];
    const nativeSources: {
      close: ReturnType<typeof vi.fn>;
      onerror?: VoidFunction;
      onopen?: VoidFunction;
    }[] = [];
    const sockets: {
      close: ReturnType<typeof vi.fn>;
      onclose?: VoidFunction;
      onmessage?: (event: Readonly<{ data: string }>) => void;
      onopen?: VoidFunction;
      send(data: string): void;
      readonly sent: string[];
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource() {
        const source = { close: vi.fn() };
        nativeSources.push(source);
        return source;
      },
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership(request) {
        return new Promise((resolve, reject) => controls.push({ reject, request, resolve }));
      },
      timers: timers.port,
      webSocket() {
        const sent: string[] = [];
        const socket = { close: vi.fn(), send: (data: string) => sent.push(data), sent };
        sockets.push(socket);
        return socket;
      },
    });
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports,
    });
    const current = authorization(transport);
    const mutation = model();
    const committedAuthorization = mutation.authorization;
    const committedCapability = mutation.capability;
    const commit = vi.fn(() => {
      mutation.position = 1n;
      mutation.authorization = Object.freeze({ generation: "staged" });
      mutation.capability = Object.freeze({ generation: "staged" });
      mutation.effects.event();
      mutation.effects.refresh();
      mutation.effects.render();
      mutation.effects.signal();
      return "committed" as const;
    });
    const states = vi.fn();
    pool.subscribe(
      current,
      {
        envelope: vi.fn(),
        reauthorize: vi.fn(() => Promise.reject(new Error("unexpected_reauthorization"))),
        state: states,
      },
      Object.freeze({ commit, discard: vi.fn(), proof: "complete_replay", subscription: current }),
    );

    if (transport === "sse") nativeSources[0]?.onopen?.();
    else sockets[0]?.onopen?.();
    await settle();

    if (transport === "sse") {
      const control = controls[0];
      if (control === undefined) throw new Error("missing_sse_control");
      if (ending === "rejected") control.reject(new Error("rejected"));
      else if (ending === "timeout") timers.fire(5_000);
      else if (ending === "transport_loss" || ending === "late") {
        nativeSources[0]?.onerror?.();
        if (ending === "late") control.resolve(sseAcknowledgment(control.request));
      } else {
        const foreign = controls[0];
        if (foreign === undefined) throw new Error("missing_foreign_control");
        foreign.resolve({ ...sseAcknowledgment(foreign.request), controlNonce: "foreign-control" });
      }
    } else {
      const socket = sockets[0];
      const request = JSON.parse(socket?.sent[0] ?? "null") as Record<string, unknown>;
      const acknowledgment = canonicalize({
        control_nonce: String(request["control_nonce"]),
        descriptor_binding: String(request["descriptor_binding"]),
        kind: "membership_authenticated",
        stream: String(request["stream"]),
        subscription: String(request["subscription"]),
        transport_generation: Number(request["transport_generation"]),
      });
      if (ending === "timeout") timers.fire(5_000);
      else if (ending === "transport_loss" || ending === "late") {
        socket?.onclose?.();
        if (ending === "late") socket?.onmessage?.({ data: acknowledgment });
      } else {
        socket?.onmessage?.({
          data:
            ending === "foreign"
              ? canonicalize({
                  control_nonce: String(request["control_nonce"]),
                  descriptor_binding: String(request["descriptor_binding"]),
                  kind: "membership_authenticated",
                  stream: "foreign-stream",
                  subscription: String(request["subscription"]),
                  transport_generation: Number(request["transport_generation"]),
                })
              : canonicalize({ kind: "membership_rejected" }),
        });
      }
    }
    await settle();

    expect(commit).not.toHaveBeenCalled();
    expect(mutation.position).toBe(0n);
    expect(mutation.authorization).toBe(committedAuthorization);
    expect(mutation.capability).toBe(committedCapability);
    for (const effect of Object.values(mutation.effects)) expect(effect).not.toHaveBeenCalled();
    expect(states).not.toHaveBeenCalledWith("current");
    pool.dispose();
    expect(timers.pending.size).toBe(0);
  });

  it("rejects reuse of a settled SSE acknowledgment on a replacement membership", async () => {
    const timers = new Timers();
    const controls: {
      request: Parameters<BrowserAsyncTransportOptions["sseMembership"]>[0];
      resolve(value: unknown): void;
    }[] = [];
    const nativeSources: {
      close: ReturnType<typeof vi.fn>;
      onerror?: VoidFunction;
      onopen?: VoidFunction;
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource() {
        const source = { close: vi.fn() };
        nativeSources.push(source);
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
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports,
    });
    const current = authorization("sse");
    const initialCommit = vi.fn(() => "committed" as const);
    const successorCommit = vi.fn(() => "committed" as const);
    const states = vi.fn();
    pool.subscribe(
      current,
      {
        envelope: vi.fn(),
        reauthorize: vi.fn(() =>
          Promise.resolve(
            Object.freeze({
              commit: successorCommit,
              discard: vi.fn(),
              proof: "authoritative_no_tail" as const,
              subscription: Object.freeze({ ...current, descriptorBinding: "binding-successor" }),
            }),
          ),
        ),
        state: states,
      },
      Object.freeze({
        commit: initialCommit,
        discard: vi.fn(),
        proof: "complete_replay",
        subscription: current,
      }),
    );
    nativeSources[0]?.onopen?.();
    await settle();
    const first = controls[0];
    if (first === undefined) throw new Error("missing_initial_sse_control");
    const settledAcknowledgment = sseAcknowledgment(first.request);
    first.resolve(settledAcknowledgment);
    await settle();
    expect(initialCommit).toHaveBeenCalledOnce();

    nativeSources[0]?.onerror?.();
    timers.fire(50);
    await settle();
    nativeSources[1]?.onopen?.();
    await settle();
    const replacement = controls[1];
    if (replacement === undefined) throw new Error("missing_replacement_sse_control");
    replacement.resolve(settledAcknowledgment);
    await settle();

    expect(initialCommit).toHaveBeenCalledOnce();
    expect(successorCommit).not.toHaveBeenCalled();
    expect(states.mock.calls.filter(([state]) => state === "current")).toHaveLength(1);
    pool.dispose();
    expect(timers.pending.size).toBe(0);
  });

  it("treats a duplicate WebSocket acknowledgment as inert after one exact commit", async () => {
    const timers = new Timers();
    const sockets: {
      close: ReturnType<typeof vi.fn>;
      onmessage?: (event: Readonly<{ data: string }>) => void;
      onopen?: VoidFunction;
      send(data: string): void;
      readonly sent: string[];
    }[] = [];
    const transports = new BrowserAsyncTransportPorts({
      eventSource: vi.fn<BrowserAsyncTransportOptions["eventSource"]>(),
      fetch: vi.fn<typeof globalThis.fetch>(),
      membershipTimeoutMs: 5_000,
      sseMembership: vi.fn<BrowserAsyncTransportOptions["sseMembership"]>(),
      timers: timers.port,
      webSocket() {
        const sent: string[] = [];
        const socket = { close: vi.fn(), send: (data: string) => sent.push(data), sent };
        sockets.push(socket);
        return socket;
      },
    });
    const pool = new DocumentConnectionPool({
      handshakeScheduler: new OriginHandshakeScheduler(),
      randomness: { number: () => 0.5 },
      timers: timers.port,
      transports,
    });
    const current = authorization("websocket");
    const commit = vi.fn(() => "committed" as const);
    const states = vi.fn();
    pool.subscribe(
      current,
      { envelope: vi.fn(), reauthorize: vi.fn(), state: states },
      Object.freeze({
        commit,
        discard: vi.fn(),
        proof: "complete_replay",
        subscription: current,
      }),
    );
    const socket = sockets[0];
    socket?.onopen?.();
    const request = JSON.parse(socket?.sent[0] ?? "null") as Record<string, unknown>;
    const acknowledgment = canonicalize({
      control_nonce: String(request["control_nonce"]),
      descriptor_binding: String(request["descriptor_binding"]),
      kind: "membership_authenticated",
      stream: String(request["stream"]),
      subscription: String(request["subscription"]),
      transport_generation: Number(request["transport_generation"]),
    });
    socket?.onmessage?.({ data: acknowledgment });
    await settle();
    socket?.onmessage?.({ data: acknowledgment });
    expect(commit).toHaveBeenCalledOnce();
    expect(states.mock.calls.filter(([state]) => state === "current")).toHaveLength(1);
    pool.dispose();
    expect(timers.pending.size).toBe(0);
  });
});
