import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import type { AsyncEnvelopeDispatcher } from "../src/async-updates/dispatch.js";
import { AsyncDocumentQueueBudget, AsyncSubscription } from "../src/async-updates/subscription.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";

const SUBSCRIPTION = "subscription-lifecycle-001";

function authorization(): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 3n, sequence: 10n }),
    descriptorBinding: "descriptor-lifecycle-001",
    document: Object.freeze({
      authorizationScope: "document-lifecycle",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([]),
    expiresAt: 5_000,
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
    stream: "orders",
    subscriptionId: SUBSCRIPTION,
  });
}

function envelope(sequence: number, kind: "error" | "heartbeat" | "refresh"): string {
  return canonicalize({
    payload:
      kind === "refresh"
        ? { kind, name: "refresh" }
        : kind === "error"
          ? { code: "backpressure", kind }
          : { kind },
    position: { epoch: "3", sequence: String(sequence) },
    protocol_version: 1,
    stream: "orders",
    subscription: SUBSCRIPTION,
  });
}

describe("browser logical async subscription", () => {
  it("requires an exact successor before current and fences duplicates and gaps", () => {
    const dispatch: AsyncEnvelopeDispatcher = { dispatch: () => "observed" };
    const current = new AsyncSubscription(authorization(), dispatch, { now: () => 1_000 });
    current.connected();
    expect(current.state()).toBe("connecting");
    expect(current.receive(envelope(11, "heartbeat"))).toBe("applied");
    expect(current.state()).toBe("current");
    expect(current.receive(envelope(11, "heartbeat"))).toBe("duplicate");
    expect(current.receive(envelope(13, "heartbeat"))).toBe("gap");
    expect(current.state()).toBe("degraded");
  });

  it("commits a refresh only after its scheduler completion succeeds", () => {
    let complete: ((outcome: "succeeded" | "failed" | "canceled" | "retired") => void) | undefined;
    const dispatch: AsyncEnvelopeDispatcher = {
      dispatch: vi.fn<AsyncEnvelopeDispatcher["dispatch"]>((_value, completion) => {
        complete = completion;
        return "queued" as const;
      }),
    };
    const current = new AsyncSubscription(authorization(), dispatch, { now: () => 1_000 });
    current.connected();
    expect(current.receive(envelope(11, "refresh"))).toBe("pending");
    expect(current.position().sequence).toBe(10n);
    complete?.("succeeded");
    expect(current.position().sequence).toBe(11n);
    expect(current.state()).toBe("current");
  });

  it("expires closed on the exclusive authority boundary", () => {
    const current = new AsyncSubscription(
      authorization(),
      { dispatch: () => "observed" },
      { now: () => 5_000 },
    );
    expect(() => current.receive(envelope(11, "heartbeat"))).toThrow("async_membership_expired");
    expect(current.state()).toBe("degraded");
  });

  it("reports real queued bytes, events, and refresh state without exposing payloads", () => {
    const documentQueue = new AsyncDocumentQueueBudget();
    const completions: ((outcome: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
    const observations: {
      queuedBytes: number;
      queuedEvents: number;
      queuedRefreshes: number;
      inFlightRefreshes: number;
    }[] = [];
    const current = new AsyncSubscription(
      authorization(),
      {
        dispatch: (_value, completion) => {
          if (completion !== undefined) completions.push(completion);
          return "queued";
        },
      },
      { now: () => 1_000 },
      undefined,
      undefined,
      (observation) => observations.push(observation),
      documentQueue,
    );
    current.connected();
    const queuedRefresh = envelope(12, "refresh");
    const queuedPresentation = envelope(13, "heartbeat");
    expect(current.receive(envelope(11, "refresh"))).toBe("pending");
    expect(current.receive(queuedRefresh)).toBe("pending");
    expect(current.receive(queuedPresentation)).toBe("pending");
    expect(observations[observations.length - 1]).toEqual({
      queuedBytes:
        new TextEncoder().encode(queuedRefresh).byteLength +
        new TextEncoder().encode(queuedPresentation).byteLength,
      queuedEvents: 2,
      queuedRefreshes: 1,
      inFlightRefreshes: 1,
    });
    expect(JSON.stringify(observations)).not.toContain("heartbeat");
    completions.shift()?.("succeeded");
    completions.shift()?.("succeeded");
    expect(observations[observations.length - 1]).toEqual({
      queuedBytes: 0,
      queuedEvents: 0,
      queuedRefreshes: 0,
      inFlightRefreshes: 0,
    });
    expect(documentQueue.current()).toEqual({ queuedBytes: 0, queuedEvents: 0 });
  });

  it("releases and observes the queued tail immediately when typed error interrupts drain", () => {
    let completion:
      ((outcome: "succeeded" | "failed" | "canceled" | "retired") => void) | undefined;
    const documentQueue = new AsyncDocumentQueueBudget();
    const observations: {
      queuedBytes: number;
      queuedEvents: number;
      queuedRefreshes: number;
      inFlightRefreshes: number;
    }[] = [];
    const current = new AsyncSubscription(
      authorization(),
      {
        dispatch: (_value, candidate) => {
          if (candidate !== undefined) completion = candidate;
          return "queued";
        },
      },
      { now: () => 1_000 },
      undefined,
      undefined,
      (observation) => observations.push(observation),
      documentQueue,
    );
    current.connected();
    expect(current.receive(envelope(11, "refresh"))).toBe("pending");
    expect(current.receive(envelope(12, "heartbeat"))).toBe("pending");
    expect(current.receive(envelope(13, "error"))).toBe("pending");
    expect(current.receive(envelope(14, "heartbeat"))).toBe("pending");

    completion?.("succeeded");

    expect(current.state()).toBe("degraded");
    expect(current.position().sequence).toBe(13n);
    expect(observations[observations.length - 1]).toEqual({
      inFlightRefreshes: 0,
      queuedBytes: 0,
      queuedEvents: 0,
      queuedRefreshes: 0,
    });
    expect(documentQueue.current()).toEqual({ queuedBytes: 0, queuedEvents: 0 });
  });

  it("releases the document reservation when an active refresh is canceled", () => {
    let completion:
      ((outcome: "succeeded" | "failed" | "canceled" | "retired") => void) | undefined;
    const documentQueue = new AsyncDocumentQueueBudget();
    const current = new AsyncSubscription(
      authorization(),
      {
        dispatch: (_value, candidate) => {
          if (candidate !== undefined) completion = candidate;
          return "queued";
        },
      },
      { now: () => 1_000 },
      undefined,
      undefined,
      undefined,
      documentQueue,
    );
    current.connected();
    expect(current.receive(envelope(11, "refresh"))).toBe("pending");
    expect(current.receive(envelope(12, "heartbeat"))).toBe("pending");
    expect(documentQueue.current()).toMatchObject({ queuedEvents: 1 });

    completion?.("canceled");

    expect(current.state()).toBe("degraded");
    expect(documentQueue.current()).toEqual({ queuedBytes: 0, queuedEvents: 0 });
  });
});
