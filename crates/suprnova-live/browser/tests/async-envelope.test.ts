import { describe, expect, it } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import {
  decodeAsyncEnvelope,
  inspectAsyncEnvelopeSubscription,
} from "../src/async-updates/envelope.js";
import type { AuthorizedLogicalSubscription } from "../src/async-updates/types.js";

const SUBSCRIPTION = "subscription-envelope-001";

function membership(): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 7n, sequence: 40n }),
    descriptorBinding: "descriptor-envelope-001",
    document: Object.freeze({
      authorizationScope: "document-envelope",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([
      Object.freeze({
        cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
        maximumFanout: 1,
        name: "orders.updated",
        order: "per_source_sequence" as const,
        payloadContract: "orders.updated.v1",
        schema: "json" as const,
        source: "stream" as const,
        targets: Object.freeze(["self"]),
        version: 1,
      }),
    ]),
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
      maximumAttempts: 4,
      maximumDelayMs: 30_000,
      minimumDelayMs: 250,
    }),
    stream: "orders",
    subscriptionId: SUBSCRIPTION,
  });
}

function encoded(payload: Readonly<Record<string, JsonValue>>, sequence = 41): string {
  return canonicalize({
    payload,
    position: { epoch: "7", sequence: String(sequence) },
    protocol_version: 1,
    stream: "orders",
    subscription: SUBSCRIPTION,
  });
}

describe("browser async envelope boundary", () => {
  it("decodes only the registered presentation event and exact membership", () => {
    const value = encoded({
      event: "orders.updated",
      kind: "browser_event",
      payload: { order: 42 },
      schema_version: 1,
      target: "self",
    });
    expect(inspectAsyncEnvelopeSubscription(value)).toBe(SUBSCRIPTION);
    expect(decodeAsyncEnvelope(value, membership())).toMatchObject({
      position: { epoch: 7n, sequence: 41n },
      stream: "orders",
      subscriptionId: SUBSCRIPTION,
    });
  });

  it("rejects noncanonical, extra-key, cross-membership, and unsupported payload input", () => {
    const valid = JSON.parse(encoded({ kind: "heartbeat" })) as Record<string, unknown>;
    expect(() => decodeAsyncEnvelope(JSON.stringify(valid, null, 2), membership())).toThrow(
      "async_envelope_noncanonical",
    );
    expect(() =>
      decodeAsyncEnvelope(canonicalize({ ...valid, action: "forbidden" }), membership()),
    ).toThrow("async_envelope_invalid");
    expect(() =>
      decodeAsyncEnvelope(
        canonicalize({ ...valid, subscription: "subscription-envelope-999" }),
        membership(),
      ),
    ).toThrow("async_subscription_mismatch");
    expect(() => decodeAsyncEnvelope(encoded({ kind: "action" }), membership())).toThrow(
      "async_payload_unsupported",
    );
  });

  it("enforces the bounded canonical envelope and payload limits", () => {
    expect(() =>
      decodeAsyncEnvelope(
        encoded({
          event: "orders.updated",
          kind: "browser_event",
          payload: "x".repeat(33 * 1024),
          schema_version: 1,
          target: "self",
        }),
        membership(),
      ),
    ).toThrow(/async_(?:envelope_invalid|payload_too_large)/u);
  });
});
