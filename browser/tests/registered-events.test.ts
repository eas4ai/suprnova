import { describe, expect, it, vi } from "vitest";

import type { AsyncRegisteredEventContract } from "../src/async-updates/types.js";
import {
  type GuardedRegisteredEventTarget,
  RegisteredEventAuthority,
  type RegisteredEventTargetResolver,
} from "../src/islands/registered-events.js";

function contract(
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

function registration(events: readonly AsyncRegisteredEventContract[]) {
  return Object.freeze({ descriptorBinding: "signed-binding-v1", events });
}

function guarded(target: EventTarget, current = () => true): GuardedRegisteredEventTarget {
  return Object.freeze({
    current,
    dispatch: (event: Event) => target.dispatchEvent(event),
  });
}

function resolver(target: EventTarget, current = () => true): RegisteredEventTargetResolver {
  return {
    current,
    event: (type) => ({ type }) as Event,
    targets: () => [guarded(target)],
  };
}

describe("core registered-event authority", () => {
  it("rejects unknown names, schema drift, forbidden scope, and forged capabilities", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const capability = authority.replace(owner, registration([contract()]), resolver(target));

    expect(
      authority.dispatch(owner, capability, {
        event: "unknown.event",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 2,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "document",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(owner, Object.freeze({}) as typeof capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    expect(dispatchEvent).not.toHaveBeenCalled();
  });

  it("invalidates a stale registration and fails a retired owner closed", () => {
    const authority = new RegisteredEventAuthority();
    let active = true;
    const owner = {};
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const first = authority.replace(
      owner,
      registration([contract()]),
      resolver(target, () => active),
    );
    const second = authority.replace(
      owner,
      registration([contract({ version: 2 })]),
      resolver(target, () => active),
    );

    expect(
      authority.dispatch(owner, first, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    active = false;
    expect(
      authority.dispatch(owner, second, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 2,
        target: "self",
      }),
    ).toBe("retired");
  });

  it("keeps source and contract validation in core", () => {
    const authority = new RegisteredEventAuthority();
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;

    expect(() =>
      authority.replace(
        {},
        registration([contract({ source: "component" as "stream" })]),
        resolver(target),
      ),
    ).toThrow("registered_event_authority_invalid");
    expect(() =>
      authority.replace(
        {},
        registration([contract({ maximumFanout: 1, targets: ["self", "document"] })]),
        resolver(target),
      ),
    ).toThrow("registered_event_authority_invalid");
  });

  it("snapshots mutable registration input before issuing authority", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const mutableTargets = ["self"];
    const mutable = { ...contract(), targets: mutableTargets, version: 1 };
    const capability = authority.replace(owner, registration([mutable]), resolver(target));
    mutable.version = 2;
    mutableTargets[0] = "document";

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("dispatched");
  });

  it("rejects accessors, inherited fields, sparse or oversized arrays, and hostile descriptor traps", () => {
    const authority = new RegisteredEventAuthority();
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const bindingGetter = vi.fn(() => "signed-binding-v1");
    const accessorRegistration = Object.create(null) as Record<string, unknown>;
    Object.defineProperty(accessorRegistration, "descriptorBinding", {
      enumerable: true,
      get: bindingGetter,
    });
    Object.defineProperty(accessorRegistration, "events", {
      enumerable: true,
      value: [contract()],
    });

    expect(() => authority.replace({}, accessorRegistration as never, resolver(target))).toThrow(
      "registered_event_authority_invalid",
    );
    expect(bindingGetter).not.toHaveBeenCalled();

    const inherited = Object.create({ descriptorBinding: "signed-binding-v1" }) as {
      descriptorBinding: string;
      events: readonly AsyncRegisteredEventContract[];
    };
    inherited.events = [contract()];
    expect(() => authority.replace({}, inherited, resolver(target))).toThrow(
      "registered_event_authority_invalid",
    );

    const sparse = Array<AsyncRegisteredEventContract>(2);
    sparse[0] = contract();
    expect(() => authority.replace({}, registration(sparse), resolver(target))).toThrow(
      "registered_event_authority_invalid",
    );
    expect(() =>
      authority.replace(
        {},
        registration(Array.from({ length: 65 }, () => contract())),
        resolver(target),
      ),
    ).toThrow("registered_event_authority_invalid");

    const descriptorTrap = vi.fn(() => {
      throw new Error("secret-proxy-trap");
    });
    const hostile = new Proxy(registration([contract()]), {
      getOwnPropertyDescriptor: descriptorTrap,
    });
    expect(() => authority.replace({}, hostile, resolver(target))).toThrow(
      "registered_event_authority_invalid",
    );
    expect(descriptorTrap).toHaveBeenCalledOnce();
  });

  it("reads each own data property once and stores only the immutable snapshot", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const mutableTargets = ["self"];
    const mutableContract = { ...contract(), targets: mutableTargets };
    const reads = new Map<PropertyKey, number>();
    const observed = new Proxy(mutableContract, {
      getOwnPropertyDescriptor(object, property) {
        reads.set(property, (reads.get(property) ?? 0) + 1);
        return Reflect.getOwnPropertyDescriptor(object, property);
      },
    });
    const capability = authority.replace(owner, registration([observed]), resolver(target));

    mutableContract.version = 2;
    mutableTargets[0] = "document";
    expect(Math.max(...reads.values())).toBe(1);
    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("dispatched");
  });

  it("enforces resolved fanout without trusting a caller-supplied maximum", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const capability = authority.replace(owner, registration([contract()]), {
      ...resolver(target),
      targets: () => [guarded(target), guarded(target)],
    });

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("fanout_exceeded");
  });

  it("does not dispatch to a target that retired during core resolution", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    let active = true;
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const capability = authority.replace(owner, registration([contract({ schema: "boolean" })]), {
      ...resolver(target, () => active),
      targets: () => {
        active = false;
        return [guarded(target)];
      },
    });

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: "not-a-boolean",
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: true,
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("retired");
    expect(dispatchEvent).not.toHaveBeenCalled();
  });

  it("stops fanout when an earlier DOM listener retires the island", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const secondDispatch = vi.fn(() => true);
    const first = {
      dispatchEvent() {
        authority.retire(owner);
        return true;
      },
    } as unknown as EventTarget;
    const second = { dispatchEvent: secondDispatch } as unknown as EventTarget;
    const capability = authority.replace(owner, registration([contract({ maximumFanout: 2 })]), {
      ...resolver(first),
      targets: () => [guarded(first), guarded(second)],
    });

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toEqual({
      delivered: 1,
      kind: "partially_dispatched",
      reason: "capability_rotated",
      skipped: 1,
    });
    expect(secondDispatch).not.toHaveBeenCalled();
  });

  it("revalidates every guarded target and reports partial fanout when a prior listener retires a later target", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    let secondCurrent = true;
    const firstDispatch = vi.fn(() => {
      secondCurrent = false;
      return true;
    });
    const secondDispatch = vi.fn(() => true);
    const capability = authority.replace(owner, registration([contract({ maximumFanout: 2 })]), {
      current: () => true,
      event: (type) => ({ type }) as Event,
      targets: () =>
        [
          Object.freeze({ current: () => true, dispatch: firstDispatch }),
          Object.freeze({ current: () => secondCurrent, dispatch: secondDispatch }),
        ] as never,
    });

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toEqual({
      delivered: 1,
      kind: "partially_dispatched",
      reason: "target_retired",
      skipped: 1,
    });
    expect(firstDispatch).toHaveBeenCalledOnce();
    expect(secondDispatch).not.toHaveBeenCalled();
  });

  it("stops synchronous delivery cycles under the registered policy", () => {
    const authority = new RegisteredEventAuthority();
    const owner = {};
    const capabilities: ReturnType<RegisteredEventAuthority["replace"]>[] = [];
    const forged = Object.freeze({}) as ReturnType<RegisteredEventAuthority["replace"]>;
    const nested = vi.fn();
    const target = {
      dispatchEvent() {
        nested(
          authority.dispatch(owner, capabilities[0] ?? forged, {
            event: "orders.updated",
            payload: {},
            schemaVersion: 1,
            target: "self",
          }),
        );
        return true;
      },
    } as unknown as EventTarget;
    const capability = authority.replace(owner, registration([contract()]), resolver(target));
    capabilities.push(capability);

    expect(
      authority.dispatch(owner, capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("dispatched");
    expect(nested).toHaveBeenCalledWith("rejected");
  });
});
