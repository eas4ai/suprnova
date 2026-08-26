import { describe, expect, it, vi } from "vitest";

import { canonicalize, type JsonValue } from "../src/canonical.js";
import { AsyncDispatcher } from "../src/async-updates/dispatch.js";
import { decodeAsyncEnvelope } from "../src/async-updates/envelope.js";
import { AsyncSubscription } from "../src/async-updates/subscription.js";
import type {
  AsyncRegisteredEventContract,
  AuthorizedLogicalSubscription,
} from "../src/async-updates/types.js";
import type {
  RegisteredBrowserEventCapability,
  RuntimeFeatureIslandPort,
} from "../src/features/contract.js";
import { IslandRecord } from "../src/islands/record.js";
import {
  RegisteredEventAuthority,
  type RegisteredEventTargetResolver,
} from "../src/islands/registered-events.js";

const SUBSCRIPTION_ID = "c3Vic2NyaXB0aW9uLTAwMQ";

function eventContract(
  overrides: Partial<AsyncRegisteredEventContract> = {},
): AsyncRegisteredEventContract {
  return Object.freeze({
    cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
    maximumFanout: 1,
    name: "orders.updated",
    order: "per_source_sequence" as const,
    payloadContract: "orders.updated.v1",
    schema: "json" as const,
    source: "stream" as const,
    targets: Object.freeze(["self"]),
    version: 1,
    ...overrides,
  });
}

function authorization(
  overrides: Partial<AuthorizedLogicalSubscription> = {},
): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 4n, sequence: 40n }),
    descriptorBinding: "descriptor-binding-001",
    document: Object.freeze({
      authorizationScope: "document-scope-001",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([eventContract()]),
    expiresAt: 10_000,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
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
    ...overrides,
  });
}

function encoded(
  payload: Readonly<Record<string, JsonValue>>,
  sequence = 41,
  subscription = SUBSCRIPTION_ID,
): string {
  return canonicalize({
    payload,
    position: { epoch: "4", sequence: String(sequence) },
    protocol_version: 1,
    stream: "orders",
    subscription,
  });
}

function envelope(
  payload: Readonly<Record<string, JsonValue>>,
  membership = authorization(),
  sequence = 41,
) {
  return decodeAsyncEnvelope(encoded(payload, sequence), membership);
}

function fakeCapability(): RegisteredBrowserEventCapability {
  return Object.freeze({}) as RegisteredBrowserEventCapability;
}

function fakePort(overrides: Partial<RuntimeFeatureIslandPort> = {}) {
  const element = Object.freeze({ nodeType: 1 }) as unknown as Element;
  const calls = {
    action: vi.fn(),
    call: vi.fn(),
    commit: vi.fn(),
    effect: vi.fn(),
    event: vi.fn(() => "dispatched" as const),
    morph: vi.fn(),
    refresh: vi.fn(() => "queued" as const),
    signal: vi.fn((_element: Element, _name: string, value: JsonValue) => value),
    stateWrite: vi.fn(),
  };
  const port: RuntimeFeatureIslandPort = {
    authorizeRegisteredEvents: () => fakeCapability(),
    dispatchRegisteredEvent: calls.event,
    element,
    enqueueFreshRender: calls.refresh,
    identity: Object.freeze({
      component: "fixture.orders",
      documentKey: "document-orders",
      slot: "orders-slot",
    }),
    onDispose: vi.fn(),
    proposeUploadHandle: () => "retired",
    queryDirectiveOwnership: () => Object.freeze([]),
    writePresentationSignal: calls.signal,
    ...overrides,
  };
  return { calls, element, port };
}

function resolver(
  targets: () => readonly EventTarget[] | "fanout_exceeded",
  current = () => true,
): RegisteredEventTargetResolver {
  return {
    current,
    event: (type, detail) => ({ detail, type }) as unknown as Event,
    targets: () => targets(),
  };
}

describe("closed asynchronous presentation dispatcher", () => {
  it("rejects every authority-writing payload without actions, morphs, or commits", () => {
    const { calls, port } = fakePort();
    const dispatcher = new AsyncDispatcher(port, fakeCapability);
    const forbidden = [
      { action: "delete_all", kind: "action" },
      { kind: "effect", name: "run_javascript" },
      { kind: "call", name: "private.scheduler" },
      { html: "<script>owned()</script>", kind: "html" },
      { kind: "snapshot", revision: "999" },
      { kind: "fragment", value: "<p>not authority</p>" },
      { kind: "component_state", value: { admin: true } },
      { kind: "revision", value: "999" },
    ];

    for (const payload of forbidden) {
      expect(() =>
        dispatcher.dispatch(
          Object.freeze({
            payload,
            position: Object.freeze({ epoch: 4n, sequence: 41n }),
            protocolVersion: 1,
            stream: "orders",
            subscriptionId: SUBSCRIPTION_ID,
          }) as never,
        ),
      ).toThrow("unsupported_async_payload");
    }

    for (const authorityWrite of [
      calls.action,
      calls.call,
      calls.commit,
      calls.effect,
      calls.event,
      calls.morph,
      calls.refresh,
      calls.signal,
      calls.stateWrite,
    ]) {
      expect(authorityWrite).not.toHaveBeenCalled();
    }
  });

  it("maps the complete validated union to only three productive presentation paths", () => {
    const capability = fakeCapability();
    const { calls, element, port } = fakePort();
    const dispatcher = new AsyncDispatcher(port, () => capability);

    expect(dispatcher.dispatch(envelope({ kind: "refresh", name: "refresh" }))).toBe("queued");
    expect(
      dispatcher.dispatch(
        envelope({
          event: "orders.updated",
          kind: "browser_event",
          payload: { count: 1 },
          schema_version: 1,
          target: "self",
        }),
      ),
    ).toBe("dispatched");
    expect(
      dispatcher.dispatch(
        envelope({ kind: "presentation_signal", name: "completion_percent", value: 50 }),
      ),
    ).toBe("signal_updated");
    expect(dispatcher.dispatch(envelope({ kind: "heartbeat" }))).toBe("observed");
    expect(dispatcher.dispatch(envelope({ kind: "complete", reason: "stream_completed" }))).toBe(
      "closed",
    );
    expect(dispatcher.dispatch(envelope({ code: "backpressure", kind: "error" }))).toBe("degraded");

    expect(calls.refresh).toHaveBeenCalledExactlyOnceWith("stream");
    expect(calls.event).toHaveBeenCalledExactlyOnceWith(capability, {
      event: "orders.updated",
      payload: { count: 1 },
      schemaVersion: 1,
      target: "self",
    });
    expect(calls.signal).toHaveBeenCalledExactlyOnceWith(element, "completion_percent", 50);
  });

  it("fails closed for forged, stale, retired, wrong-island, wrong-scope, and over-fanout event authority", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const wrongOwner = {};
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const registration = Object.freeze({
      descriptorBinding: "signed-binding-v1",
      events: Object.freeze([eventContract()]),
    });
    const capability = authority.replace(
      owner,
      registration,
      resolver(() => [target]),
    );
    const corePort = (dispatchOwner: object, selected: () => RegisteredBrowserEventCapability) => {
      const { port } = fakePort({
        dispatchRegisteredEvent: (candidate, event) =>
          authority.dispatch(dispatchOwner, candidate, event),
      });
      return new AsyncDispatcher(port, selected);
    };
    const registeredEvent = envelope({
      event: "orders.updated",
      kind: "browser_event",
      payload: { count: 1 },
      schema_version: 1,
      target: "self",
    });

    expect(corePort(wrongOwner, () => capability).dispatch(registeredEvent)).toBe("rejected");
    expect(corePort(owner, fakeCapability).dispatch(registeredEvent)).toBe("rejected");

    const successor = authority.replace(
      owner,
      registration,
      resolver(() => [target]),
    );
    expect(corePort(owner, () => capability).dispatch(registeredEvent)).toBe("rejected");
    authority.retire(owner);
    expect(corePort(owner, () => successor).dispatch(registeredEvent)).toBe("rejected");

    const fanoutCapability = authority.replace(
      owner,
      registration,
      resolver(() => [target, target]),
    );
    expect(corePort(owner, () => fanoutCapability).dispatch(registeredEvent)).toBe("rejected");

    const scopeMembership = authorization({
      events: Object.freeze([eventContract({ maximumFanout: 2, targets: ["self", "document"] })]),
    });
    const wrongScope = envelope(
      {
        event: "orders.updated",
        kind: "browser_event",
        payload: { count: 1 },
        schema_version: 1,
        target: "document",
      },
      scopeMembership,
    );
    const selfOnlyCapability = authority.replace(
      owner,
      registration,
      resolver(() => [target]),
    );
    expect(corePort(owner, () => selfOnlyCapability).dispatch(wrongScope)).toBe("rejected");
    expect(dispatchEvent).not.toHaveBeenCalled();
  });

  it("revalidates cycle policy in core immediately before DOM dispatch", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const nested = vi.fn();
    let dispatcher: AsyncDispatcher | null = null;
    const event = envelope({
      event: "orders.updated",
      kind: "browser_event",
      payload: { count: 1 },
      schema_version: 1,
      target: "self",
    });
    const target = {
      dispatchEvent() {
        nested(dispatcher?.dispatch(event));
        return true;
      },
    } as unknown as EventTarget;
    const capability = authority.replace(
      owner,
      Object.freeze({
        descriptorBinding: "signed-binding-v1",
        events: Object.freeze([eventContract()]),
      }),
      resolver(() => [target]),
    );
    const { port } = fakePort({
      dispatchRegisteredEvent: (candidate, candidateEvent) =>
        authority.dispatch(owner, candidate, candidateEvent),
    });
    dispatcher = new AsyncDispatcher(port, () => capability);

    expect(dispatcher.dispatch(event)).toBe("dispatched");
    expect(nested).toHaveBeenCalledExactlyOnceWith("rejected");
  });

  it("rejects undeclared or retired signal writes before committing sequence state", () => {
    const signal = vi.fn((_element: Element, _name: string, value: JsonValue) => value);
    const { port } = fakePort({ writePresentationSignal: signal });
    const dispatcher = new AsyncDispatcher(port, fakeCapability);
    const subscription = new AsyncSubscription(authorization(), dispatcher, { now: () => 1_000 });

    expect(() =>
      subscription.receive(
        encoded({ kind: "presentation_signal", name: "undeclared_signal", value: 1 }),
      ),
    ).toThrow("async_payload_unregistered");
    expect(signal).not.toHaveBeenCalled();
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });

    signal.mockImplementation(() => {
      throw new Error("signal_scope_retired");
    });
    expect(
      subscription.receive(
        encoded({ kind: "presentation_signal", name: "completion_percent", value: 1 }),
      ),
    ).toBe("dispatch_failed");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(subscription.state()).toBe("degraded");
  });

  it("closes or degrades only the exact decoded subscription lifecycle", () => {
    const { port } = fakePort();
    const completed = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
    );
    completed.receive(encoded({ kind: "heartbeat" }, 41));
    expect(() =>
      completed.receive(
        encoded(
          { kind: "complete", reason: "stream_completed" },
          42,
          "Zm9yZWlnbi1zdWJzY3JpcHRpb24",
        ),
      ),
    ).toThrow("async_subscription_mismatch");
    expect(completed.state()).toBe("current");
    expect(completed.receive(encoded({ kind: "complete", reason: "stream_completed" }, 42))).toBe(
      "applied",
    );
    expect(completed.state()).toBe("closed");

    const failed = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
    );
    expect(failed.receive(encoded({ code: "backpressure", kind: "error" }, 41))).toBe("applied");
    expect(failed.state()).toBe("degraded");
  });

  it("uses the existing per-island scheduler with one in-flight and one queued refresh", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(
      element,
      Object.freeze({
        component: "fixture.orders",
        documentKey: "document-orders",
        instanceId: "MDEyMzQ1Njc4OTo7PD0-Pw",
        lazyComplete: false,
        protocolMinimum: 2,
        revision: 7n,
        runtimeContract: 1,
        slot: "orders-slot",
        snapshot: Object.freeze({}),
        snapshotForm: "instance" as const,
      }),
    );
    const { port } = fakePort({
      element,
      enqueueFreshRender: (reason) => record.enqueueFreshRender(reason),
    });
    const dispatcher = new AsyncDispatcher(port, fakeCapability);
    const refresh = envelope({ kind: "refresh", name: "refresh" });

    expect(dispatcher.dispatch(refresh)).toBe("queued");
    const first = record.scheduler.ready()[0];
    if (first === undefined) throw new Error("missing fresh-render scheduler ticket");
    expect(record.scheduler.start(first)).toBe("accepted");
    for (let index = 0; index < 1_000; index += 1) dispatcher.dispatch(refresh);

    expect(record.scheduler.snapshot()).toMatchObject({ inFlight: 1, queued: 1 });
    record.dispose();
  });
});
