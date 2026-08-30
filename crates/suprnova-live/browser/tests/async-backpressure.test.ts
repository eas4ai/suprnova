import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import {
  OriginHandshakeScheduler,
  type DocumentTransportPort,
  type DocumentTransportConnectRequest,
} from "../src/async-updates/connections.js";
import type { AsyncEnvelopeDispatcher } from "../src/async-updates/dispatch.js";
import { AsyncDocumentQueueBudget, AsyncSubscription } from "../src/async-updates/subscription.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";

const SUBSCRIPTION = "subscription-pressure-001";

function authorization(
  subscriptionId = SUBSCRIPTION,
  stream = "orders",
): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: "descriptor-pressure-001",
    document: Object.freeze({
      authorizationScope: "document-pressure",
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
      maximumAttempts: 3,
      maximumDelayMs: 5_000,
      minimumDelayMs: 250,
    }),
    stream,
    subscriptionId,
  });
}

function envelope(sequence: number, refresh = false, membership = authorization()): string {
  return canonicalize({
    payload: refresh ? { kind: "refresh", name: "refresh" } : { kind: "heartbeat" },
    position: { epoch: "1", sequence: String(sequence) },
    protocol_version: 1,
    stream: membership.stream,
    subscription: membership.subscriptionId,
  });
}

describe("browser async bounded pressure", () => {
  it("admits no more than eight concurrent same-origin handshakes and releases fairly", () => {
    const scheduler = new OriginHandshakeScheduler();
    const releases: VoidFunction[] = [];
    const started: number[] = [];
    for (let index = 0; index < 16; index += 1) {
      scheduler.schedule("https://app.example.test", (release) => {
        started.push(index);
        releases.push(release);
      });
    }
    expect(scheduler.active("https://app.example.test")).toBe(8);
    while (releases.length > 0) releases.shift()?.();
    expect(started).toEqual(Array.from({ length: 16 }, (_, index) => index));
    expect(scheduler.active("https://app.example.test")).toBe(0);
  });

  it("coalesces adjacent refresh pressure and commits only after completion", () => {
    const completions: ((outcome: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
    const dispatchSpy = vi.fn<AsyncEnvelopeDispatcher["dispatch"]>((_value, completion) => {
      if (completion !== undefined) completions.push(completion);
      return "queued" as const;
    });
    const dispatch: AsyncEnvelopeDispatcher = { dispatch: dispatchSpy };
    const current = new AsyncSubscription(authorization(), dispatch, { now: () => 1_000 });
    current.connected();
    expect(current.receive(envelope(1, true))).toBe("pending");
    for (let sequence = 2; sequence <= 64; sequence += 1) {
      expect(current.receive(envelope(sequence, true))).toBe("pending");
    }
    expect(dispatchSpy).toHaveBeenCalledOnce();
    completions[0]?.("succeeded");
    expect(current.position().sequence).toBe(1n);
    expect(dispatchSpy).toHaveBeenCalledTimes(2);
    completions[1]?.("succeeded");
    expect(current.position().sequence).toBe(64n);
    expect(current.state()).toBe("current");
  });

  it("atomically enforces the 64-event document cap across subscriptions", () => {
    const documentQueue = new AsyncDocumentQueueBudget();
    const firstAuthorization = authorization("subscription-pressure-first", "orders-first");
    const secondAuthorization = authorization("subscription-pressure-second", "orders-second");
    const firstLifecycle = vi.fn();
    const secondLifecycle = vi.fn();
    const held: AsyncEnvelopeDispatcher = { dispatch: () => "queued" };
    const first = new AsyncSubscription(
      firstAuthorization,
      held,
      { now: () => 1_000 },
      undefined,
      firstLifecycle,
      undefined,
      documentQueue,
    );
    const second = new AsyncSubscription(
      secondAuthorization,
      held,
      { now: () => 1_000 },
      undefined,
      secondLifecycle,
      undefined,
      documentQueue,
    );
    first.connected();
    second.connected();
    expect(first.receive(envelope(1, true, firstAuthorization))).toBe("pending");
    expect(second.receive(envelope(1, true, secondAuthorization))).toBe("pending");
    for (let sequence = 2; sequence <= 33; sequence += 1) {
      expect(first.receive(envelope(sequence, false, firstAuthorization))).toBe("pending");
      expect(second.receive(envelope(sequence, false, secondAuthorization))).toBe("pending");
    }
    expect(documentQueue.current()).toMatchObject({ queuedEvents: 64 });

    expect(first.receive(envelope(34, false, firstAuthorization))).toBe("dispatch_failed");
    expect(firstLifecycle).toHaveBeenLastCalledWith({
      kind: "dispatch_failed",
      reason: "resource_exhausted",
    });
    expect(secondLifecycle).not.toHaveBeenCalled();
    expect(documentQueue.current()).toMatchObject({ queuedEvents: 32 });

    second.close();
    expect(documentQueue.current()).toEqual({ queuedBytes: 0, queuedEvents: 0 });
  });

  it("accepts the exact document byte cap and rejects one byte over without leaking", () => {
    const documentQueue = new AsyncDocumentQueueBudget();
    expect(documentQueue.reserve(8, 256 * 1024)).toBe(true);
    expect(documentQueue.current()).toEqual({
      queuedBytes: 256 * 1024,
      queuedEvents: 8,
    });
    expect(documentQueue.reserve(1, 1)).toBe(false);
    expect(documentQueue.current()).toEqual({
      queuedBytes: 256 * 1024,
      queuedEvents: 8,
    });
    documentQueue.release(8, 256 * 1024);
    expect(documentQueue.current()).toEqual({ queuedBytes: 0, queuedEvents: 0 });
  });

  it("keeps physical transport failure callbacks typed and one-way", () => {
    const failed = vi.fn<DocumentTransportConnectRequest["failed"]>();
    const port: DocumentTransportPort = {
      close: vi.fn(),
      subscribe: () => Object.freeze({ kind: "rejected" as const, reason: "capacity" as const }),
      unsubscribe: vi.fn(),
    };
    void port.subscribe(authorization());
    failed("transport_lost");
    expect(failed).toHaveBeenCalledExactlyOnceWith("transport_lost");
  });
});
