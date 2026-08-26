import { describe, expect, it, vi } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import { AsyncSubscription } from "../src/async-updates/subscription.js";
import type {
  AsyncDispatchPort,
  AsyncPayload,
  AuthorizedLogicalSubscription,
  StreamPosition,
} from "../src/async-updates/types.js";

const SUBSCRIPTION_ID = "c3Vic2NyaXB0aW9uLTAwMQ";

function position(epoch: bigint, sequence: bigint): StreamPosition {
  return Object.freeze({ epoch, sequence });
}

function authorized(baseline: StreamPosition = position(4n, 40n)): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline,
    descriptorBinding: "descriptor-binding-001",
    document: Object.freeze({
      authorizationScope: "document-scope-001",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([
      Object.freeze({
        maximumFanout: 8,
        name: "orders.updated",
        schema: "json" as const,
        targets: Object.freeze(["self"]),
        version: 1,
      }),
    ]),
    expiresAt: 10_000,
    heartbeatTimeoutMs: 30_000,
    presentationSignals: Object.freeze([
      Object.freeze({ name: "completion_percent", schema: "u64" as const }),
    ]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 30_000,
      minimumDelayMs: 250,
    }),
    stream: "orders",
    subscriptionId: SUBSCRIPTION_ID,
  });
}

function envelope(at: StreamPosition, payload: AsyncPayload): string {
  return canonicalize({
    payload: payload as unknown as JsonValue,
    position: { epoch: String(at.epoch), sequence: String(at.sequence) },
    protocol_version: 1,
    stream: "orders",
    subscription: SUBSCRIPTION_ID,
  });
}

function fixture() {
  const applied: AsyncPayload[] = [];
  const browserEvent = vi.fn((event: Extract<AsyncPayload, { kind: "browser_event" }>) => {
    applied.push(event);
    return true;
  });
  const dispatch: AsyncDispatchPort = {
    browserEvent,
    presentationSignal: vi.fn((signal: Extract<AsyncPayload, { kind: "presentation_signal" }>) => {
      applied.push(signal);
      return true;
    }),
    refresh: vi.fn((refresh: Extract<AsyncPayload, { kind: "refresh" }>) => {
      applied.push(refresh);
      return true;
    }),
  };
  const subscription = new AsyncSubscription(authorized(), dispatch, { now: () => 1_000 });
  return { applied, browserEvent, dispatch, subscription };
}

describe("browser asynchronous subscription continuity", () => {
  it("cannot claim current on initial connect without an exact successor proof", () => {
    const { applied, subscription } = fixture();

    subscription.connected();
    expect(subscription.state()).toBe("connecting");
    expect(
      subscription.receive(
        envelope(position(4n, 41n), Object.freeze({ kind: "refresh", name: "refresh" })),
      ),
    ).toBe("applied");
    expect(subscription.state()).toBe("current");

    expect(
      subscription.receive(
        envelope(
          position(4n, 43n),
          Object.freeze({ kind: "presentation_signal", name: "completion_percent", value: 50 }),
        ),
      ),
    ).toBe("gap");
    expect(subscription.state()).toBe("degraded");
    expect(applied).toEqual([Object.freeze({ kind: "refresh", name: "refresh" })]);
  });

  it("ignores duplicate and stale positions without redispatching them", () => {
    const { applied, subscription } = fixture();
    const first = envelope(
      position(4n, 41n),
      Object.freeze({ kind: "presentation_signal", name: "completion_percent", value: 50 }),
    );

    expect(subscription.receive(first)).toBe("applied");
    expect(subscription.receive(first)).toBe("duplicate");
    expect(
      subscription.receive(envelope(position(3n, 999n), Object.freeze({ kind: "heartbeat" }))),
    ).toBe("stale");
    expect(applied).toHaveLength(1);
  });

  it("prevalidates a complete replay transcript before dispatching any member", () => {
    const { applied, subscription } = fixture();
    expect(
      subscription.receive(envelope(position(4n, 43n), Object.freeze({ kind: "heartbeat" }))),
    ).toBe("gap");

    const malformedSecond = canonicalize({
      payload: { kind: "html", html: "<p>not authority</p>" },
      position: { epoch: "4", sequence: "43" },
      protocol_version: 1,
      stream: "orders",
      subscription: SUBSCRIPTION_ID,
    });
    expect(() =>
      subscription.receiveReplay([
        envelope(position(4n, 41n), Object.freeze({ kind: "heartbeat" })),
        envelope(position(4n, 42n), Object.freeze({ kind: "heartbeat" })),
        malformedSecond,
      ]),
    ).toThrow("async_payload_unsupported");
    expect(subscription.position()).toEqual(position(4n, 40n));
    expect(applied).toEqual([]);
  });

  it("rejects a replay transcript whose aggregate bytes exceed the document bound", () => {
    const membership = {
      ...authorized(),
      presentationSignals: Object.freeze([
        Object.freeze({ name: "message", schema: "string" as const }),
      ]),
    };
    const subscription = new AsyncSubscription(
      membership,
      {
        browserEvent: () => true,
        presentationSignal: () => true,
        refresh: () => true,
      },
      { now: () => 1_000 },
    );
    const transcript = Array.from({ length: 9 }, (_, index) =>
      canonicalize({
        payload: { kind: "presentation_signal", name: "message", value: "x".repeat(30_000) },
        position: { epoch: "4", sequence: String(41 + index) },
        protocol_version: 1,
        stream: "orders",
        subscription: SUBSCRIPTION_ID,
      }),
    );

    expect(() => subscription.receiveReplay(transcript)).toThrow("async_replay_too_large");
    expect(subscription.position()).toEqual(position(4n, 40n));
  });

  it("claims current after a complete validated reconnect replay and not socket open", () => {
    const { subscription } = fixture();
    subscription.receive(envelope(position(4n, 41n), Object.freeze({ kind: "heartbeat" })));
    subscription.transportLost();
    expect(subscription.state()).toBe("reconnecting");
    subscription.connected();
    expect(subscription.state()).toBe("connecting");

    expect(
      subscription.receiveReplay([
        envelope(position(4n, 42n), Object.freeze({ kind: "heartbeat" })),
        envelope(position(4n, 43n), Object.freeze({ kind: "heartbeat" })),
      ]),
    ).toEqual({ applied: 2, through: position(4n, 43n) });
    expect(subscription.state()).toBe("current");
  });

  it("degrades on heartbeat loss or authorization uncertainty without applying late data", () => {
    const { applied, subscription } = fixture();
    subscription.receive(envelope(position(4n, 41n), Object.freeze({ kind: "heartbeat" })));
    subscription.heartbeatLost();
    expect(subscription.state()).toBe("degraded");
    expect(
      subscription.receive(
        envelope(
          position(4n, 42n),
          Object.freeze({ kind: "presentation_signal", name: "completion_percent", value: 51 }),
        ),
      ),
    ).toBe("continuity_required");
    subscription.authorizationUncertain();
    expect(subscription.state()).toBe("degraded");
    expect(applied).toEqual([]);
  });

  it("validates registered event, signal, and exact membership before dispatch", () => {
    const { browserEvent, subscription } = fixture();
    const event = envelope(
      position(4n, 41n),
      Object.freeze({
        event: "orders.updated",
        kind: "browser_event",
        payload: Object.freeze({ count: 1 }),
        schema_version: 1,
        target: "self",
      }),
    );

    expect(subscription.receive(event)).toBe("applied");
    expect(browserEvent).toHaveBeenCalledOnce();
    expect(() =>
      subscription.receive(
        canonicalize({
          payload: { kind: "heartbeat" },
          position: { epoch: "4", sequence: "42" },
          protocol_version: 1,
          stream: "other-stream",
          subscription: SUBSCRIPTION_ID,
        }),
      ),
    ).toThrow("async_stream_mismatch");
    expect(() =>
      subscription.receive(
        envelope(
          position(4n, 42n),
          Object.freeze({
            event: "orders.deleted",
            kind: "browser_event",
            payload: Object.freeze({ count: 1 }),
            schema_version: 1,
            target: "self",
          }),
        ),
      ),
    ).toThrow("async_payload_unregistered");
    expect(browserEvent).toHaveBeenCalledOnce();
  });

  it("retains the applied position but requires proof after restored authorization", () => {
    const { subscription } = fixture();
    subscription.receive(envelope(position(4n, 41n), Object.freeze({ kind: "heartbeat" })));
    const restored = {
      ...authorized(),
      baseline: position(4n, 41n),
      descriptorBinding: "descriptor-binding-restored",
      expiresAt: 20_000,
    };

    subscription.reauthorize(restored);

    expect(subscription.position()).toEqual(position(4n, 41n));
    expect(subscription.state()).toBe("connecting");
    expect(
      subscription.receive(envelope(position(4n, 42n), Object.freeze({ kind: "heartbeat" }))),
    ).toBe("continuity_required");
    expect(
      subscription.receiveReplay([
        envelope(position(4n, 42n), Object.freeze({ kind: "heartbeat" })),
      ]),
    ).toEqual({ applied: 1, through: position(4n, 42n) });
    expect(subscription.state()).toBe("current");
  });
});
