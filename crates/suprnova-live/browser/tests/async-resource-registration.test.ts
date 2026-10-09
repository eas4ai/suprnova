import { describe, expect, it, vi } from "vitest";

import { AsyncDocumentOwner } from "../src/async-updates/feature.js";
import type {
  DocumentMembershipOutcome,
  DocumentTransportPort,
  LogicalSubscriptionSink,
} from "../src/async-updates/connections.js";
import type { PollEnvironment, PollPolicy } from "../src/async-updates/poll.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";
import type {
  AsyncRuntimeIslandPort,
  RuntimeFeatureDocumentContext,
} from "../src/features/contract.js";
import { type CoreResourceKind, ResourceLedgerImpl } from "../src/lifecycle/resources.js";

const POLL_POLICY: PollPolicy = Object.freeze({
  initial: "wait",
  intervalMs: 30_000,
  jitterRatio: 0,
  mode: "poll_only",
  visibility: "visible",
});

function context(ledger: ResourceLedgerImpl): RuntimeFeatureDocumentContext {
  return Object.freeze({
    diagnose: vi.fn(),
    onDispose: vi.fn(),
    trackResource: (kind: CoreResourceKind, dispose: VoidFunction) => ledger.add(kind, dispose),
  });
}

function pollPort(): AsyncRuntimeIslandPort {
  return {
    element: Object.freeze({}) as Element,
    enqueueFreshRender: vi.fn(() => "queued" as const),
  } as unknown as AsyncRuntimeIslandPort;
}

function pollEnvironment(
  unsubscribe: VoidFunction,
  subscribe = vi.fn(() => unsubscribe),
): PollEnvironment {
  return Object.freeze({
    isOnline: () => true,
    isVisible: () => true,
    subscribe,
  });
}

function authorization(): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 1n, sequence: 0n }),
    descriptorBinding: "binding-001",
    document: Object.freeze({
      authorizationScope: "document-scope",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([]),
    expiresAt: 20_000,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 5_000,
    presentationSignals: Object.freeze([]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 1,
      maximumDelayMs: 1_000,
      minimumDelayMs: 1_000,
    }),
    stream: "orders",
    subscriptionId: "subscription-001",
  });
}

function sink(): LogicalSubscriptionSink {
  return Object.freeze({
    envelope: vi.fn(),
    reauthorize: vi.fn(() => {
      throw new Error("unexpected_reauthorization");
    }),
    state: vi.fn(),
  });
}

describe("async resource registration rollback", () => {
  it("unsubscribes exactly once when listener registration exceeds document capacity", () => {
    const ledger = new ResourceLedgerImpl({ maxResources: 1 });
    const retained = ledger.add("controller", () => undefined);
    const unsubscribe = vi.fn();
    const subscribe = vi.fn(() => unsubscribe);
    const environment = pollEnvironment(unsubscribe, subscribe);
    const owner = new AsyncDocumentOwner(context(ledger), {
      clock: { now: () => 100 },
      pollEnvironment: environment,
      randomness: { number: () => 0.5 },
      timers: {
        clearTimeout: vi.fn(),
        timeout: vi.fn(() => 1),
      },
    });

    expect(() => owner.activatePoll(pollPort(), POLL_POLICY)).toThrow("resource_ledger_capacity");
    owner.dispose();
    retained.dispose();

    expect(subscribe).toHaveBeenCalledOnce();
    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(Object.values(ledger.counts()).every((count) => count === 0)).toBe(true);
  });

  it("clears exactly once when timer registration exceeds document capacity", () => {
    const ledger = new ResourceLedgerImpl({ maxResources: 1 });
    const unsubscribe = vi.fn();
    const clearTimeout = vi.fn();
    const owner = new AsyncDocumentOwner(context(ledger), {
      clock: { now: () => 100 },
      pollEnvironment: pollEnvironment(unsubscribe),
      randomness: { number: () => 0.5 },
      timers: {
        clearTimeout,
        timeout: vi.fn(() => 41),
      },
    });

    expect(() => owner.activatePoll(pollPort(), POLL_POLICY)).toThrow("resource_ledger_capacity");
    owner.dispose();

    expect(clearTimeout).toHaveBeenCalledExactlyOnceWith(41);
    expect(unsubscribe).toHaveBeenCalledOnce();
    expect(Object.values(ledger.counts()).every((count) => count === 0)).toBe(true);
  });

  it.each([1, 2])(
    "closes an acquired SSE port exactly once when capacity %i rejects its resource set",
    (maxResources) => {
      const ledger = new ResourceLedgerImpl({ maxResources });
      const clearTimeout = vi.fn();
      const close = vi.fn();
      const rawPort: DocumentTransportPort = Object.freeze({
        close,
        subscribe: vi.fn((subscription: AuthorizedLogicalSubscription): DocumentMembershipOutcome =>
          Object.freeze({
            descriptorBinding: subscription.descriptorBinding,
            kind: "authenticated" as const,
            stream: subscription.stream,
            subscriptionId: subscription.subscriptionId,
            transportGeneration: 1,
          }),
        ),
        unsubscribe: vi.fn(),
      });
      const eventSource = vi.fn(() => rawPort);
      const owner = new AsyncDocumentOwner(context(ledger), {
        authority: { authorize: () => authorization() },
        clock: { now: () => 100 },
        randomness: { number: () => 0.5 },
        timers: {
          clearTimeout,
          timeout: vi.fn(() => 73),
        },
        transports: {
          eventSource,
          webSocket: () => {
            throw new Error("unexpected_websocket");
          },
        },
      });

      owner.subscribe(authorization(), sink());
      owner.dispose();

      expect(eventSource).toHaveBeenCalledOnce();
      expect(close).toHaveBeenCalledExactlyOnceWith("document_retired");
      expect(Object.values(ledger.counts()).every((count) => count === 0)).toBe(true);
    },
  );
});
