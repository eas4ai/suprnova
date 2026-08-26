import { describe, expect, it, vi } from "vitest";

import type { AsyncRegisteredEventContract } from "../src/async-updates/types.js";
import {
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

function resolver(target: EventTarget, current = () => true): RegisteredEventTargetResolver {
  return {
    current,
    event: (type) => ({ type }) as Event,
    targets: () => [target],
  };
}

describe("core registered-event authority", () => {
  it("rejects unknown names, schema drift, forbidden scope, and forged capabilities", () => {
    const authority = new RegisteredEventAuthority();
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const capability = authority.replace({}, registration([contract()]), resolver(target));

    expect(
      authority.dispatch(capability, {
        event: "unknown.event",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 2,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "document",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(Object.freeze({}) as typeof capability, {
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
      authority.dispatch(first, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    active = false;
    expect(
      authority.dispatch(second, {
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
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const mutableTargets = ["self"];
    const mutable = { ...contract(), targets: mutableTargets, version: 1 };
    const capability = authority.replace({}, registration([mutable]), resolver(target));
    mutable.version = 2;
    mutableTargets[0] = "document";

    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("dispatched");
  });

  it("enforces resolved fanout without trusting a caller-supplied maximum", () => {
    const authority = new RegisteredEventAuthority();
    const target = { dispatchEvent: vi.fn(() => true) } as unknown as EventTarget;
    const capability = authority.replace({}, registration([contract()]), {
      ...resolver(target),
      targets: () => [target, target],
    });

    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("fanout_exceeded");
  });

  it("does not dispatch to a target that retired during core resolution", () => {
    const authority = new RegisteredEventAuthority();
    const dispatchEvent = vi.fn(() => true);
    const target = { dispatchEvent } as unknown as EventTarget;
    const capability = authority.replace({}, registration([contract({ schema: "boolean" })]), {
      ...resolver(target),
      targets: () => [],
    });

    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: "not-a-boolean",
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("rejected");
    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: true,
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("no_target");
    expect(dispatchEvent).not.toHaveBeenCalled();
  });

  it("stops synchronous delivery cycles under the registered policy", () => {
    const authority = new RegisteredEventAuthority();
    const capabilities: ReturnType<RegisteredEventAuthority["replace"]>[] = [];
    const forged = Object.freeze({}) as ReturnType<RegisteredEventAuthority["replace"]>;
    const nested = vi.fn();
    const target = {
      dispatchEvent() {
        nested(
          authority.dispatch(capabilities[0] ?? forged, {
            event: "orders.updated",
            payload: {},
            schemaVersion: 1,
            target: "self",
          }),
        );
        return true;
      },
    } as unknown as EventTarget;
    const capability = authority.replace({}, registration([contract()]), resolver(target));
    capabilities.push(capability);

    expect(
      authority.dispatch(capability, {
        event: "orders.updated",
        payload: {},
        schemaVersion: 1,
        target: "self",
      }),
    ).toBe("dispatched");
    expect(nested).toHaveBeenCalledWith("rejected");
  });
});
