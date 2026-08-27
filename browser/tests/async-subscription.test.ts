import { describe, expect, it, vi } from "vitest";

import { canonicalize } from "../src/canonical.js";
import type { AsyncEnvelopeDispatcher } from "../src/async-updates/dispatch.js";
import { AsyncSubscription } from "../src/async-updates/subscription.js";
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

function envelope(sequence: number, kind: "heartbeat" | "refresh"): string {
  return canonicalize({
    payload: kind === "refresh" ? { kind, name: "refresh" } : { kind },
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
});
