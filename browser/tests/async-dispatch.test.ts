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
  FreshRenderDisposition,
  RegisteredBrowserEventCapability,
  AsyncRuntimeIslandPort,
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
      Object.freeze({ name: "completion_percent", schema: "u64" as const, scope: "root-scope" }),
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

function fakePort(overrides: Partial<AsyncRuntimeIslandPort> = {}) {
  const element = Object.freeze({ nodeType: 1 }) as unknown as Element;
  const calls = {
    action: vi.fn(),
    call: vi.fn(),
    commit: vi.fn(),
    effect: vi.fn(),
    event: vi.fn(() => "dispatched" as const),
    morph: vi.fn(),
    refresh: vi.fn(() => "queued" as const),
    signal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    stateWrite: vi.fn(),
  };
  const port: AsyncRuntimeIslandPort = {
    consumeRegisteredEventCapability: () => fakeCapability(),
    dispatchRegisteredEvent: calls.event,
    element,
    enqueueFreshRender: calls.refresh,
    identity: Object.freeze({
      component: "fixture.orders",
      documentKey: "document-orders",
      slot: "orders-slot",
    }),
    onDispose: vi.fn(),
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
    targets: () => {
      const resolved = targets();
      return resolved === "fanout_exceeded"
        ? resolved
        : resolved.map((target) =>
            Object.freeze({
              current: () => true,
              dispatch: (event: Event) => target.dispatchEvent(event),
            }),
          );
    },
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
    const { calls, port } = fakePort();
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
        envelope({
          kind: "presentation_signal",
          name: "completion_percent",
          scope: "root-scope",
          value: 50,
        }),
      ),
    ).toBe("signal_updated");
    expect(dispatcher.dispatch(envelope({ kind: "heartbeat" }))).toBe("observed");
    expect(dispatcher.dispatch(envelope({ kind: "complete", reason: "stream_completed" }))).toBe(
      "closed:stream_completed",
    );
    expect(dispatcher.dispatch(envelope({ code: "backpressure", kind: "error" }))).toBe(
      "degraded:backpressure",
    );

    expect(calls.refresh).toHaveBeenCalledExactlyOnceWith(
      "stream",
      expect.any(Function),
      SUBSCRIPTION_ID,
    );
    expect(calls.event).toHaveBeenCalledExactlyOnceWith(capability, {
      event: "orders.updated",
      payload: { count: 1 },
      schemaVersion: 1,
      target: "self",
    });
    expect(calls.signal).toHaveBeenCalledExactlyOnceWith("root-scope", "completion_percent", 50);
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

  it("rechecks source and target authority after the event factory runs", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    let current = true;
    const dispatch = vi.fn(() => true);
    const capability = authority.replace(
      owner,
      Object.freeze({
        descriptorBinding: "signed-binding-v1",
        events: Object.freeze([eventContract()]),
      }),
      {
        current: () => current,
        event: () => {
          current = false;
          return Object.freeze({}) as unknown as Event;
        },
        targets: () =>
          Object.freeze([
            Object.freeze({
              current: () => current,
              dispatch,
            }),
          ]),
      },
    );

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: { count: 1 },
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("retired");
    expect(dispatch).not.toHaveBeenCalled();
  });

  it("reports an observable delivered prefix without committing the partial sequence", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    let secondCurrent = true;
    const first = vi.fn(() => {
      secondCurrent = false;
      return true;
    });
    const second = vi.fn(() => true);
    const capability = authority.replace(
      owner,
      Object.freeze({
        descriptorBinding: "signed-binding-v1",
        events: Object.freeze([eventContract({ maximumFanout: 2 })]),
      }),
      {
        current: () => true,
        event: () => Object.freeze({}) as unknown as Event,
        targets: () =>
          Object.freeze([
            Object.freeze({ current: () => true, dispatch: first }),
            Object.freeze({ current: () => secondCurrent, dispatch: second }),
          ]),
      },
    );
    const { port } = fakePort({
      dispatchRegisteredEvent: (candidate, event) => authority.dispatch(owner, candidate, event),
    });
    const lifecycle = vi.fn();
    const subscription = new AsyncSubscription(
      authorization({ events: Object.freeze([eventContract({ maximumFanout: 2 })]) }),
      new AsyncDispatcher(port, () => capability),
      { now: () => 1_000 },
      undefined,
      lifecycle,
    );

    expect(
      subscription.receive(
        encoded({
          event: "orders.updated",
          kind: "browser_event",
          payload: { count: 1 },
          schema_version: 1,
          target: "self",
        }),
      ),
    ).toBe("dispatch_failed");
    expect(first).toHaveBeenCalledOnce();
    expect(second).not.toHaveBeenCalled();
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(lifecycle).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "dispatch_failed", reason: "presentation_partial" }),
    );
    expect(subscription.receive(encoded({ kind: "heartbeat" }, 42))).toBe("continuity_required");
    expect(first).toHaveBeenCalledOnce();
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
  });

  it("rejects undeclared or retired signal writes before committing sequence state", () => {
    const signal = vi.fn((_scope: string, _name: string, value: JsonValue) => value);
    const { port } = fakePort({ writePresentationSignal: signal });
    const dispatcher = new AsyncDispatcher(port, fakeCapability);
    const subscription = new AsyncSubscription(authorization(), dispatcher, { now: () => 1_000 });

    expect(() =>
      subscription.receive(
        encoded({
          kind: "presentation_signal",
          name: "undeclared_signal",
          scope: "root-scope",
          value: 1,
        }),
      ),
    ).toThrow("async_payload_unregistered");
    expect(signal).not.toHaveBeenCalled();
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });

    expect(() =>
      subscription.receive(
        encoded({
          kind: "presentation_signal",
          name: "completion_percent",
          scope: "foreign-scope",
          value: 1,
        }),
      ),
    ).toThrow("async_payload_unregistered");
    expect(signal).not.toHaveBeenCalled();
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });

    signal.mockImplementation(() => {
      throw new Error("signal_scope_retired");
    });
    expect(
      subscription.receive(
        encoded({
          kind: "presentation_signal",
          name: "completion_percent",
          scope: "root-scope",
          value: 1,
        }),
      ),
    ).toBe("dispatch_failed");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(subscription.state()).toBe("degraded");
  });

  it.each(["_root", "-root", ".root", ":root", "root/scope"])(
    "rejects signal scope outside the shared grammar: %s",
    (scope) => {
      const subscription = new AsyncSubscription(
        authorization(),
        new AsyncDispatcher(fakePort().port, fakeCapability),
        { now: () => 1_000 },
      );
      expect(() =>
        subscription.receive(
          encoded({
            kind: "presentation_signal",
            name: "completion_percent",
            scope,
            value: 1,
          }),
        ),
      ).toThrow("async_payload_unregistered");
      expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    },
  );

  it.each([
    "Progress",
    "1progress",
    "_progress",
    "progress/value",
    `a${"z".repeat(64)}`,
    "prøgress",
  ])("rejects signal name outside the shared lowercase-first grammar: %s", (name) => {
    const hostile = authorization({
      presentationSignals: Object.freeze([
        Object.freeze({ name, schema: "u64", scope: "root-scope" }),
      ]),
    });
    expect(() =>
      decodeAsyncEnvelope(
        encoded({ kind: "presentation_signal", name, scope: "root-scope", value: 1 }),
        hostile,
      ),
    ).toThrow("async_payload_unregistered");
  });

  it("accepts the exact 64-byte signal-name boundary", () => {
    const name = `a${"z".repeat(63)}`;
    const membership = authorization({
      presentationSignals: Object.freeze([
        Object.freeze({ name, schema: "u64", scope: "root-scope" }),
      ]),
    });
    expect(
      decodeAsyncEnvelope(
        encoded({ kind: "presentation_signal", name, scope: "root-scope", value: 1 }),
        membership,
      ).payload,
    ).toMatchObject({ kind: "presentation_signal", name });
  });

  it("rejects generic JSON and floating-point presentation-signal schemas", () => {
    for (const schema of ["json", "f64"] as const) {
      const hostile = authorization({
        presentationSignals: Object.freeze([
          Object.freeze({ name: "completion_percent", schema, scope: "root-scope" }),
        ]) as never,
      });
      expect(() =>
        decodeAsyncEnvelope(
          encoded({
            kind: "presentation_signal",
            name: "completion_percent",
            scope: "root-scope",
            value: schema === "json" ? { forged: true } : 1.5,
          }),
          hostile,
        ),
      ).toThrow("async_payload_unregistered");
    }
  });

  it("binds a presentation update to the exact signed nested signal-scope identity", () => {
    const scoped = authorization({
      presentationSignals: Object.freeze([
        Object.freeze({ name: "completion_percent", schema: "u64", scope: "nested-panel" }),
      ]),
    });
    const signal = vi.fn((_scope: string, _name: string, value: JsonValue) => value);
    const { port } = fakePort({ writePresentationSignal: signal });
    const dispatcher = new AsyncDispatcher(port, fakeCapability);

    expect(
      dispatcher.dispatch(
        envelope(
          {
            kind: "presentation_signal",
            name: "completion_percent",
            scope: "nested-panel",
            value: 50,
          },
          scoped,
        ),
      ),
    ).toBe("signal_updated");
    expect(signal).toHaveBeenCalledExactlyOnceWith("nested-panel", "completion_percent", 50);
  });

  it("closes or degrades only the exact decoded subscription lifecycle", () => {
    const { port } = fakePort();
    const completedLifecycle = vi.fn();
    const completed = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
      undefined,
      completedLifecycle,
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
    expect(completedLifecycle).toHaveBeenCalledExactlyOnceWith({
      kind: "complete",
      reason: "stream_completed",
    });

    const failedLifecycle = vi.fn();
    const failed = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
      undefined,
      failedLifecycle,
    );
    expect(failed.receive(encoded({ code: "backpressure", kind: "error" }, 41))).toBe("applied");
    expect(failed.state()).toBe("degraded");
    expect(failedLifecycle).toHaveBeenCalledExactlyOnceWith({
      kind: "error",
      reason: "backpressure",
    });
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

  it("coalesces a pending refresh without reporting success before its terminal outcome", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(
      element,
      Object.freeze({
        component: "fixture.orders",
        documentKey: "document-orders-coalesced",
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
    const first = vi.fn();
    const second = vi.fn();

    expect(record.enqueueFreshRender("stream", first)).toBe("queued");
    expect(record.enqueueFreshRender("stream", second)).toBe("coalesced");
    expect(first).not.toHaveBeenCalled();
    expect(second).not.toHaveBeenCalled();
    expect(record.scheduler.snapshot()).toMatchObject({ inFlight: 0, queued: 1 });
    const queued = record.scheduler.ready()[0];
    if (queued === undefined) throw new Error("missing coalesced fresh-render ticket");
    expect(record.scheduler.start(queued)).toBe("accepted");
    expect(record.scheduler.settleTransport(queued)).toBe("accepted");
    expect(record.scheduler.beginApplication(queued)).toBe("accepted");
    expect(record.scheduler.finish(queued, "accepted")).toBe("accepted");
    expect(first).toHaveBeenCalledExactlyOnceWith("succeeded");
    expect(second).toHaveBeenCalledExactlyOnceWith("succeeded");
    record.dispose();
  });

  it("keeps coalesced refresh completion ownership bounded for one thousand admissions", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(
      element,
      Object.freeze({
        component: "fixture.orders",
        documentKey: "document-orders-bounded-completion",
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
    const completion = vi.fn();

    expect(record.enqueueFreshRender("stream", completion)).toBe("queued");
    for (let index = 0; index < 1_000; index += 1) {
      expect(record.enqueueFreshRender("stream", completion)).toBe("coalesced");
    }

    const queued = record.scheduler.ready()[0];
    if (queued === undefined) throw new Error("missing bounded fresh-render ticket");
    record.scheduler.start(queued);
    record.scheduler.settleTransport(queued);
    record.scheduler.beginApplication(queued);
    record.scheduler.finish(queued, "accepted");
    expect(completion).toHaveBeenCalledExactlyOnceWith("succeeded");
    record.dispose();
  });

  it("fails an overflowing semantic completion owner truthfully without throwing", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(
      element,
      Object.freeze({
        component: "fixture.orders",
        documentKey: "document-orders-bounded-owner-overflow",
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
    const accepted = Array.from({ length: 256 }, () => vi.fn());
    for (const [index, completion] of accepted.entries()) {
      expect(record.enqueueFreshRender("stream", completion, `subscription-${String(index)}`)).toBe(
        index === 0 ? "queued" : "coalesced",
      );
    }
    const overflow = vi.fn();
    let disposition: FreshRenderDisposition | undefined;

    expect(() => {
      disposition = record.enqueueFreshRender("stream", overflow, "subscription-overflow");
    }).not.toThrow();
    expect(disposition).toBe("exhausted");
    expect(overflow).toHaveBeenCalledExactlyOnceWith("failed");
    for (const completion of accepted) expect(completion).not.toHaveBeenCalled();
    record.dispose();
  });

  it("does not commit a partially delivered event sequence", () => {
    const { port } = fakePort({
      dispatchRegisteredEvent: () =>
        Object.freeze({
          delivered: 1,
          kind: "partially_dispatched" as const,
          reason: "target_retired" as const,
          skipped: 1,
        }),
    });
    const lifecycle = vi.fn();
    const subscription = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
      undefined,
      lifecycle,
    );

    expect(
      subscription.receive(
        encoded({
          event: "orders.updated",
          kind: "browser_event",
          payload: { count: 1 },
          schema_version: 1,
          target: "self",
        }),
      ),
    ).toBe("dispatch_failed");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(subscription.state()).toBe("degraded");
    expect(lifecycle).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "dispatch_failed", reason: "presentation_partial" }),
    );
  });

  it("never replays a partially delivered position and requires a trusted baseline that absorbs it", () => {
    const event = vi
      .fn()
      .mockReturnValueOnce(
        Object.freeze({
          delivered: 1,
          kind: "partially_dispatched" as const,
          reason: "target_retired" as const,
          skipped: 1,
        }),
      )
      .mockReturnValue("dispatched" as const);
    const { port } = fakePort({ dispatchRegisteredEvent: event });
    const subscription = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
    );
    const event41 = encoded({
      event: "orders.updated",
      kind: "browser_event",
      payload: { count: 1 },
      schema_version: 1,
      target: "self",
    });

    expect(subscription.receive(event41)).toBe("dispatch_failed");
    expect(event).toHaveBeenCalledOnce();
    expect(() => subscription.preflightReauthorization(authorization(), [event41])).toThrow(
      "async_replay_non_replayable",
    );
    expect(event).toHaveBeenCalledOnce();

    const recovered = authorization({ baseline: Object.freeze({ epoch: 4n, sequence: 41n }) });
    expect(subscription.preflightReauthorization(recovered, [])).toBe("authoritative_no_tail");
    subscription.reauthorize(recovered);
    subscription.proveAuthoritativeBaseline(recovered.baseline);
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 41n });

    expect(
      subscription.receive(
        encoded(
          {
            event: "orders.updated",
            kind: "browser_event",
            payload: { count: 2 },
            schema_version: 1,
            target: "self",
          },
          42,
        ),
      ),
    ).toBe("applied");
    expect(event).toHaveBeenCalledTimes(2);
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 42n });
  });

  it("waits for replay refresh terminals before committing the replay transcript", () => {
    const completions: ((result: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
    const signal = vi.fn((_scope: string, _name: string, value: JsonValue) => value);
    const { port } = fakePort({
      enqueueFreshRender: (_reason, observer) => {
        if (observer !== undefined) completions.push(observer);
        return "queued";
      },
      writePresentationSignal: signal,
    });
    const subscription = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
    );

    expect(
      subscription.receiveReplay([
        encoded({ kind: "refresh", name: "refresh" }, 41),
        encoded(
          {
            kind: "presentation_signal",
            name: "completion_percent",
            scope: "root-scope",
            value: 50,
          },
          42,
        ),
      ]),
    ).toBe("pending");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(signal).not.toHaveBeenCalled();

    completions[0]?.("failed");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
    expect(subscription.state()).toBe("degraded");
    expect(signal).not.toHaveBeenCalled();
  });

  it.each(["failed", "canceled", "retired"] as const)(
    "keeps a stream refresh pending until terminal %s completion and then degrades at the committed high-water mark",
    (completion) => {
      const completions: ((result: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
      const { port } = fakePort({
        enqueueFreshRender: (_reason, observer) => {
          if (observer !== undefined) completions.push(observer);
          return "queued";
        },
      });
      const subscription = new AsyncSubscription(
        authorization(),
        new AsyncDispatcher(port, fakeCapability),
        { now: () => 1_000 },
      );

      expect(subscription.receive(encoded({ kind: "refresh", name: "refresh" }, 41))).toBe(
        "pending",
      );
      expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
      const complete = completions[0];
      if (complete === undefined) throw new Error("missing stream refresh completion observer");
      complete(completion);
      expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
      expect(subscription.state()).toBe("degraded");
    },
  );

  it("reports a stream refresh successful only after the scheduler's commit-after-morph terminal outcome", () => {
    const completions: ((result: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
    const { port } = fakePort({
      enqueueFreshRender: (_reason, observer) => {
        if (observer !== undefined) completions.push(observer);
        return "queued";
      },
    });
    const subscription = new AsyncSubscription(
      authorization(),
      new AsyncDispatcher(port, fakeCapability),
      { now: () => 1_000 },
    );

    expect(subscription.receive(encoded({ kind: "refresh", name: "refresh" }, 41))).toBe("pending");
    const complete = completions[0];
    if (complete === undefined) throw new Error("missing stream refresh completion observer");
    complete("succeeded");
    expect(subscription.position()).toEqual({ epoch: 4n, sequence: 41n });
    expect(subscription.state()).toBe("current");
  });

  it.each(["transport_lost", "heartbeat_lost", "authorization_rotated"] as const)(
    "does not let late refresh success cross a newer %s generation",
    (loss) => {
      const completions: ((result: "succeeded" | "failed" | "canceled" | "retired") => void)[] = [];
      const lifecycle = vi.fn();
      const { port } = fakePort({
        enqueueFreshRender: (_reason, observer) => {
          if (observer !== undefined) completions.push(observer);
          return "queued";
        },
      });
      const subscription = new AsyncSubscription(
        authorization(),
        new AsyncDispatcher(port, fakeCapability),
        { now: () => 1_000 },
        undefined,
        lifecycle,
      );
      expect(subscription.receive(encoded({ kind: "refresh", name: "refresh" }, 41))).toBe(
        "pending",
      );
      if (loss === "transport_lost") subscription.transportLost();
      else if (loss === "heartbeat_lost") subscription.heartbeatLost();
      else subscription.reauthorize(authorization());

      completions[0]?.("succeeded");

      expect(subscription.position()).toEqual({ epoch: 4n, sequence: 40n });
      expect(subscription.state()).toBe("degraded");
      expect(lifecycle).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "dispatch_failed", reason: "refresh_canceled" }),
      );
    },
  );
});
