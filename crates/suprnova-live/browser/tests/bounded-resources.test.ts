import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BoundedOwner,
  HARD_MAX_ACTIVE_PERMITS,
  HARD_MAX_RESOURCE_BYTES,
  HARD_MAX_RESOURCE_ITEMS,
  type BoundedDisposable,
  type BoundedLease,
} from "../src/features/bounded.js";
import type {
  FeatureResourceKind,
  ResourceKind,
  ResourceLedger,
} from "../src/lifecycle/resources.js";

const BROWSER_ROOT = fileURLToPath(new URL("../", import.meta.url));

// @ts-expect-error Null cannot be a bounded-owner payload type.
export type NullPayloadOwnerMustBeRejected = BoundedOwner<null>;
// @ts-expect-error Undefined cannot be a bounded-owner payload type.
export type UndefinedPayloadOwnerMustBeRejected = BoundedOwner<undefined>;
// @ts-expect-error The runtime ledger is deliberately not generic over feature kinds.
export type FeatureLedgerMustBeRejected = ResourceLedger<FeatureResourceKind>;

function coreLedgerTypeProof(ledger: ResourceLedger): void {
  // @ts-expect-error Optional feature kinds never enter the core runtime ledger.
  ledger.add("upload", () => undefined);
}
void coreLedgerTypeProof;

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  expect(vi.getTimerCount()).toBe(0);
  vi.useRealTimers();
});

function owner<T extends NonNullable<unknown>>(
  overrides: Partial<ConstructorParameters<typeof BoundedOwner<T>>[0]> = {},
) {
  return new BoundedOwner<T>({
    maxActive: 1,
    maxBytes: 8,
    maxItems: 2,
    ...overrides,
  });
}

describe("bounded feature owner", () => {
  it("snapshots every configured limit accessor exactly once before validation", () => {
    const reads = { maxActive: 0, maxBytes: 0, maxItems: 0 };
    const limits = { maxActive: 0, maxBytes: 0, maxItems: 0 };
    Object.defineProperties(limits, {
      maxActive: {
        enumerable: true,
        get: () => (++reads.maxActive === 1 ? 1 : HARD_MAX_ACTIVE_PERMITS + 1),
      },
      maxBytes: {
        enumerable: true,
        get: () => (++reads.maxBytes === 1 ? 8 : HARD_MAX_RESOURCE_BYTES + 1),
      },
      maxItems: {
        enumerable: true,
        get: () => (++reads.maxItems === 1 ? 2 : HARD_MAX_RESOURCE_ITEMS + 1),
      },
    });
    const bounded = new BoundedOwner<string>(limits);

    expect(reads).toEqual({ maxActive: 1, maxBytes: 1, maxItems: 1 });
    expect(bounded.enqueue("full", 8)).toBe("accepted");
    expect(bounded.enqueue("over-bytes", 1)).toBe("bytes_exceeded");
    expect(bounded.dequeue()).toBe("full");
    expect(bounded.enqueue("first-item", 0)).toBe("accepted");
    expect(bounded.enqueue("second-item", 0)).toBe("accepted");
    expect(bounded.enqueue("over-items", 0)).toBe("items_exceeded");
    const lease = bounded.acquire();
    expect(lease).not.toBeNull();
    expect(bounded.acquire()).toBeNull();
    lease?.dispose();
  });

  it("does not enumerate or reread a proxy after snapshotting its limit values", () => {
    const reads = new Map<PropertyKey, number>();
    const values = new Map<PropertyKey, number>([
      ["maxActive", 1],
      ["maxBytes", 8],
      ["maxItems", 2],
    ]);
    const limits = new Proxy({} as ConstructorParameters<typeof BoundedOwner<string>>[0], {
      get: (_target, property) => {
        reads.set(property, (reads.get(property) ?? 0) + 1);
        const value = values.get(property);
        if (value === undefined) throw new Error("unexpected_limit_property");
        return reads.get(property) === 1 ? value : Number.MAX_SAFE_INTEGER;
      },
      ownKeys: () => {
        throw new Error("limits_must_not_be_enumerated");
      },
    });

    expect(() => new BoundedOwner<string>(limits)).not.toThrow();
    expect(Object.fromEntries(reads)).toEqual({ maxActive: 1, maxBytes: 1, maxItems: 1 });
  });

  it("rejects non-finite, fractional, zero, and above-ceiling limits", () => {
    const invalid = [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY];
    for (const value of invalid) {
      expect(() => owner({ maxItems: value })).toThrow("bounded_owner_limits");
      expect(() => owner({ maxBytes: value })).toThrow("bounded_owner_limits");
      expect(() => owner({ maxActive: value })).toThrow("bounded_owner_limits");
    }
    expect(() => owner({ maxItems: HARD_MAX_RESOURCE_ITEMS + 1 })).toThrow("bounded_owner_limits");
    expect(() => owner({ maxBytes: HARD_MAX_RESOURCE_BYTES + 1 })).toThrow("bounded_owner_limits");
    expect(() => owner({ maxActive: HARD_MAX_ACTIVE_PERMITS + 1 })).toThrow("bounded_owner_limits");
  });

  it("enforces queue item and byte caps without corrupting accounting", () => {
    const bounded = owner<string>();
    expect(bounded.enqueue("first", 5)).toBe("accepted");
    expect(bounded.enqueue("too-many-bytes", 4)).toBe("bytes_exceeded");
    expect(bounded.enqueue("second", 3)).toBe("accepted");
    expect(bounded.enqueue("too-many-items", 0)).toBe("items_exceeded");
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 8, queuedItems: 2 });

    for (const bytes of [-1, 0.5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(() => bounded.enqueue("invalid", bytes)).toThrow("bounded_owner_item_bytes");
    }
    expect(owner<string>().enqueue("safely-too-large", HARD_MAX_RESOURCE_BYTES + 1)).toBe(
      "bytes_exceeded",
    );
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 8, queuedItems: 2 });
  });

  it("dequeues in FIFO order and releases each byte reservation once", () => {
    const bounded = owner<string>({ maxBytes: 10, maxItems: 3 });
    bounded.enqueue("first", 2);
    bounded.enqueue("second", 3);
    bounded.enqueue("third", 5);

    expect(bounded.dequeue()).toBe("first");
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 8, queuedItems: 2 });
    expect(bounded.dequeue()).toBe("second");
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 5, queuedItems: 1 });
    expect(bounded.dequeue()).toBe("third");
    expect(bounded.dequeue()).toBeNull();
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 0, queuedItems: 0 });
  });

  it("drains the legal hard-cap FIFO without per-item array removal", () => {
    const bounded = owner<number>({
      maxBytes: 1,
      maxItems: HARD_MAX_RESOURCE_ITEMS,
    });
    for (let index = 0; index < HARD_MAX_RESOURCE_ITEMS; index += 1) {
      expect(bounded.enqueue(index, 0)).toBe("accepted");
    }

    let mismatch = -1;
    let empty: number | null;
    const originalShift = Array.prototype.shift;
    let queueShiftCalls = 0;
    Array.prototype.shift = function <Item>(this: Item[]): Item | undefined {
      const first = this[0];
      if (typeof first === "object" && first !== null && "bytes" in first && "value" in first) {
        queueShiftCalls += 1;
      }
      return originalShift.call(this) as Item | undefined;
    };
    try {
      for (let index = 0; index < HARD_MAX_RESOURCE_ITEMS; index += 1) {
        if (bounded.dequeue() !== index && mismatch < 0) mismatch = index;
      }
      empty = bounded.dequeue();
    } finally {
      Array.prototype.shift = originalShift;
    }

    expect(mismatch).toBe(-1);
    expect(empty).toBeNull();
    expect(queueShiftCalls).toBe(0);
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 0, queuedItems: 0 });
    bounded.retire();
  });

  it("rejects nullish payloads so null remains an unambiguous empty sentinel", () => {
    const bounded = owner<string>();

    expect(() => bounded.enqueue(null as never, 0)).toThrow("bounded_owner_item_value");
    expect(() => bounded.enqueue(undefined as never, 0)).toThrow("bounded_owner_item_value");
    expect(bounded.snapshot()).toMatchObject({ queuedBytes: 0, queuedItems: 0 });
    expect(bounded.dequeue()).toBeNull();
  });

  it("admits permit waiters fairly and cancellation cannot skip the FIFO head", () => {
    const bounded = owner<string>({ maxItems: 3 });
    const blocker = bounded.acquire();
    expect(blocker).not.toBeNull();
    const admitted: string[] = [];
    const leases: { first?: BoundedLease; third?: BoundedLease } = {};
    const first = bounded.requestPermit((lease) => {
      admitted.push("first");
      leases.first = lease;
    });
    const canceled = bounded.requestPermit(() => {
      admitted.push("canceled");
    });
    const third = bounded.requestPermit((lease) => {
      admitted.push("third");
      leases.third = lease;
    });

    expect(first.state()).toBe("waiting");
    expect(canceled.state()).toBe("waiting");
    expect(third.state()).toBe("waiting");
    expect(bounded.acquire()).toBeNull();
    canceled.dispose();
    blocker?.dispose();
    expect(admitted).toEqual(["first"]);
    expect(first.state()).toBe("admitted");
    expect(canceled.state()).toBe("canceled");
    expect(third.state()).toBe("waiting");

    leases.first?.dispose();
    expect(admitted).toEqual(["first", "third"]);
    expect(third.state()).toBe("admitted");
    leases.third?.dispose();
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
  });

  it("bounds queued permit waiters and releases canceled capacity immediately", () => {
    const bounded = owner<string>({ maxItems: 2 });
    const blocker = bounded.acquire();
    const first = bounded.requestPermit(() => undefined);
    const second = bounded.requestPermit(() => undefined);
    const rejected = bounded.requestPermit(() => {
      throw new Error("over-limit waiter must not run");
    });

    expect(first.state()).toBe("waiting");
    expect(second.state()).toBe("waiting");
    expect(rejected.state()).toBe("items_exceeded");
    first.dispose();
    expect(bounded.snapshot().waitingPermits).toBe(1);
    const replacement = bounded.requestPermit(() => undefined);
    expect(replacement.state()).toBe("waiting");
    blocker?.dispose();
    bounded.retire();
  });

  it("makes cancellation one-way, idempotent, and payload independent", () => {
    const bounded = owner<{ readonly secret: string }>();
    bounded.enqueue({ secret: "payload-secret-sentinel" }, 1);

    expect(bounded.isCanceled()).toBe(false);
    expect(bounded.cancel()).toBe(true);
    expect(bounded.cancel()).toBe(false);
    expect(bounded.isCanceled()).toBe(true);
    expect(JSON.stringify(bounded.snapshot())).not.toContain("payload-secret-sentinel");
    expect(bounded.dequeue()).toEqual({ secret: "payload-secret-sentinel" });
  });

  it("suspends admission and dequeue while preserving work until one resume edge", () => {
    const bounded = owner<string>();
    const hooks: string[] = [];
    bounded.track({
      dispose: () => hooks.push("dispose"),
      resume: () => hooks.push("resume"),
      suspend: () => hooks.push("suspend"),
    });
    expect(hooks).toEqual(["resume"]);

    expect(bounded.suspend()).toBe("suspended");
    expect(bounded.suspend()).toBe("suspended");
    expect(bounded.enqueue("preserved", 4)).toBe("accepted");
    expect(bounded.acquire()).toBeNull();
    expect(bounded.dequeue()).toBeNull();
    const leases: { admitted?: BoundedLease } = {};
    const request = bounded.requestPermit((admitted) => {
      leases.admitted = admitted;
    });
    expect(request.state()).toBe("waiting");
    expect(bounded.snapshot()).toMatchObject({
      queuedBytes: 4,
      queuedItems: 1,
      state: "suspended",
      waitingPermits: 1,
    });

    expect(bounded.resume()).toBe("active");
    expect(bounded.resume()).toBe("active");
    expect(hooks).toEqual(["resume", "suspend", "resume"]);
    expect(request.state()).toBe("admitted");
    expect(bounded.dequeue()).toBe("preserved");
    leases.admitted?.dispose();
  });

  it("retires queued bytes, waiters, resources, and active permits exactly once", () => {
    const bounded = owner<string>();
    expect(bounded.enqueue("queued", 4)).toBe("accepted");
    const lease = bounded.acquire();
    expect(lease).not.toBeNull();
    const waiting = bounded.requestPermit(() => {
      throw new Error("retired waiter must not run");
    });
    const disposals: string[] = [];
    bounded.track({
      dispose: () => {
        disposals.push("first");
        throw new Error("isolated disposer failure");
      },
    });
    bounded.track({ dispose: () => disposals.push("second") });

    expect(bounded.retire()).toEqual({
      drainedBytes: 4,
      drainedItems: 1,
      releasedPermits: 1,
    });
    expect(bounded.retire()).toEqual({
      drainedBytes: 0,
      drainedItems: 0,
      releasedPermits: 0,
    });
    expect(disposals).toEqual(["second", "first"]);
    expect(waiting.state()).toBe("retired");
    expect(bounded.snapshot()).toEqual({
      active: 0,
      canceled: true,
      ownedResources: 0,
      pendingResources: 0,
      queuedBytes: 0,
      queuedItems: 0,
      state: "retired",
      waitingPermits: 0,
    });

    lease?.dispose();
    lease?.dispose();
    expect(bounded.snapshot().active).toBe(0);
    expect(bounded.enqueue("late", 1)).toBe("retired");
    expect(bounded.enqueue("late-invalid", -1)).toBe("retired");
    expect(bounded.acquire()).toBeNull();
    const late = bounded.requestPermit(() => {
      throw new Error("late waiter must not run");
    });
    expect(late.state()).toBe("retired");
    expect(() => bounded.track({ dispose: () => undefined })).toThrow("bounded_owner_retired");
  });

  it("disposes leases, permit requests, and tracked resources idempotently", () => {
    const bounded = owner<string>({ maxActive: 2 });
    const hooks: string[] = [];
    const resource = bounded.track({ dispose: () => hooks.push("resource") });
    const first = bounded.acquire();
    const second = bounded.requestPermit((lease) => {
      hooks.push("admitted");
      lease.dispose();
      lease.dispose();
    });
    expect(second.state()).toBe("admitted");
    expect(bounded.snapshot().active).toBe(1);

    resource.dispose();
    resource.dispose();
    first?.dispose();
    first?.dispose();
    second.dispose();
    second.dispose();
    expect(hooks).toEqual(["admitted", "resource"]);
    expect(bounded.snapshot()).toMatchObject({ active: 0, ownedResources: 0 });
  });

  it("does not dispose an explicitly released resource again during retirement", () => {
    const bounded = owner<string>();
    const disposals: string[] = [];
    const released = bounded.track({ dispose: () => disposals.push("released") });
    bounded.track({ dispose: () => disposals.push("owned") });

    released.dispose();
    bounded.retire();
    released.dispose();

    expect(disposals).toEqual(["released", "owned"]);
    expect(bounded.snapshot().ownedResources).toBe(0);
  });

  it("bounds tracked lifecycle resources and reuses explicitly released capacity", () => {
    const bounded = owner<string>({ maxItems: 1 });
    const first = bounded.track({ dispose: () => undefined });
    expect(() => bounded.track({ dispose: () => undefined })).toThrow(
      "bounded_owner_resource_limit",
    );
    first.dispose();
    const replacement = bounded.track({ dispose: () => undefined });
    expect(bounded.snapshot().ownedResources).toBe(1);
    replacement.dispose();
  });

  it("removes cap-scale pending resources without array membership scans", () => {
    const size = 4_096;
    const half = size / 2;
    const bounded = owner<string>({ maxItems: size });
    const handles: BoundedDisposable[] = [];
    const disposals: number[] = [];
    bounded.suspend();
    for (let index = 0; index < size; index += 1) {
      handles.push(bounded.track({ dispose: () => disposals.push(index) }));
    }

    const originalIndexOf = Array.prototype.indexOf;
    const originalSplice = Array.prototype.splice;
    let resourceIndexCalls = 0;
    let resourceSpliceCalls = 0;
    Array.prototype.indexOf = function <Item>(
      this: Item[],
      searchElement: Item,
      fromIndex?: number,
    ): number {
      if (
        typeof searchElement === "object" &&
        searchElement !== null &&
        "activated" in searchElement &&
        "active" in searchElement
      ) {
        resourceIndexCalls += 1;
      }
      return originalIndexOf.call(this, searchElement, fromIndex);
    };
    Array.prototype.splice = function <Item>(
      this: Item[],
      start: number,
      deleteCount?: number,
      ...items: Item[]
    ): Item[] {
      const first = this[0];
      if (
        typeof first === "object" &&
        first !== null &&
        "activated" in first &&
        "active" in first
      ) {
        resourceSpliceCalls += 1;
      }
      return originalSplice.call(this, start, deleteCount ?? this.length - start, ...items);
    };
    try {
      for (let index = 0; index < half; index += 1) handles[index]?.dispose();
      bounded.retire();
    } finally {
      Array.prototype.indexOf = originalIndexOf;
      Array.prototype.splice = originalSplice;
    }

    expect(resourceIndexCalls).toBe(0);
    expect(resourceSpliceCalls).toBe(0);
    expect(disposals).toEqual([
      ...Array.from({ length: half }, (_, index) => index),
      ...Array.from({ length: half }, (_, index) => size - 1 - index),
    ]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 0, pendingResources: 0 });
  });

  it("deletes a reentrant deferred resource without array membership scans", () => {
    const bounded = owner<string>({ maxItems: 2 });
    const disposals: string[] = [];
    let installChild = false;
    bounded.track({
      dispose: () => disposals.push("root"),
      resume: () => {
        if (!installChild) return;
        installChild = false;
        const child = bounded.track({ dispose: () => disposals.push("child") });
        child.dispose();
      },
    });
    bounded.suspend();
    installChild = true;

    const originalIndexOf = Array.prototype.indexOf;
    const originalSplice = Array.prototype.splice;
    let resourceIndexCalls = 0;
    let resourceSpliceCalls = 0;
    Array.prototype.indexOf = function <Item>(
      this: Item[],
      searchElement: Item,
      fromIndex?: number,
    ): number {
      if (
        typeof searchElement === "object" &&
        searchElement !== null &&
        "activated" in searchElement &&
        "active" in searchElement
      ) {
        resourceIndexCalls += 1;
      }
      return originalIndexOf.call(this, searchElement, fromIndex);
    };
    Array.prototype.splice = function <Item>(
      this: Item[],
      start: number,
      deleteCount?: number,
      ...items: Item[]
    ): Item[] {
      const first = this[0];
      if (
        typeof first === "object" &&
        first !== null &&
        "activated" in first &&
        "active" in first
      ) {
        resourceSpliceCalls += 1;
      }
      return originalSplice.call(this, start, deleteCount ?? this.length - start, ...items);
    };
    try {
      bounded.resume();
    } finally {
      Array.prototype.indexOf = originalIndexOf;
      Array.prototype.splice = originalSplice;
    }

    expect(resourceIndexCalls).toBe(0);
    expect(resourceSpliceCalls).toBe(0);
    expect(disposals).toEqual(["child"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 1, pendingResources: 0 });
    bounded.retire();
    expect(disposals).toEqual(["child", "root"]);
  });

  it("serializes reentrant suspend and resume requests at lifecycle edges", () => {
    const bounded = owner<string>();
    const hooks: string[] = [];
    let reenter = false;
    bounded.track({
      dispose: () => undefined,
      resume: () => {
        hooks.push("resume");
        if (reenter) bounded.suspend();
      },
      suspend: () => {
        hooks.push("suspend");
        if (reenter) bounded.resume();
      },
    });
    hooks.length = 0;
    reenter = true;

    expect(bounded.suspend()).toBe("suspended");
    expect(bounded.snapshot().state).toBe("suspended");
    expect(hooks).toEqual(["suspend"]);
    expect(bounded.resume()).toBe("active");
    expect(bounded.snapshot().state).toBe("active");
    expect(hooks).toEqual(["suspend", "resume"]);
  });

  it("does not emit a second suspend when initial resume reenters suspension", () => {
    const bounded = owner<string>();
    const hooks: string[] = [];

    bounded.track({
      dispose: () => undefined,
      resume: () => {
        hooks.push("resume");
        bounded.suspend();
      },
      suspend: () => hooks.push("suspend"),
    });

    expect(hooks).toEqual(["resume", "suspend"]);
    expect(bounded.snapshot().state).toBe("suspended");
    bounded.retire();
  });

  it("defers resources recursively registered by an initial lifecycle callback", () => {
    const bounded = owner<string>({ maxItems: 8 });
    let callbacks = 0;
    let disposals = 0;
    const resource = (): Parameters<typeof bounded.track>[0] => ({
      dispose: () => {
        disposals += 1;
      },
      resume: () => {
        callbacks += 1;
        if (callbacks < 8) bounded.track(resource());
      },
    });

    bounded.track(resource());

    expect(callbacks).toBe(1);
    expect(bounded.snapshot().ownedResources).toBe(2);
    bounded.retire();
    expect(disposals).toBe(2);
  });

  it("advances one stable pending-resource batch before each external active track", () => {
    const bounded = owner<string>({ maxItems: 5 });
    const events: string[] = [];
    let installChild = true;
    let installGrandchild = true;

    bounded.track({
      dispose: () => undefined,
      resume: () => {
        events.push("parent");
        if (!installChild) return;
        installChild = false;
        bounded.track({
          dispose: () => undefined,
          resume: () => {
            events.push("child");
            if (!installGrandchild) return;
            installGrandchild = false;
            bounded.track({
              dispose: () => undefined,
              resume: () => events.push("grandchild"),
            });
          },
        });
      },
    });

    expect(events).toEqual(["parent"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 2, pendingResources: 1 });

    bounded.track({ dispose: () => undefined, resume: () => events.push("current") });
    expect(events).toEqual(["parent", "child", "current"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 4, pendingResources: 1 });

    bounded.track({ dispose: () => undefined, resume: () => events.push("next") });
    expect(events).toEqual(["parent", "child", "current", "grandchild", "next"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 5, pendingResources: 0 });
    bounded.retire();
  });

  it("defers track reentrancy from an active permit pump behind older pending resources", () => {
    const bounded = owner<string>({ maxItems: 4 });
    const events: string[] = [];
    bounded.track({
      dispose: () => undefined,
      resume: () => {
        events.push("parent");
        bounded.track({
          dispose: () => undefined,
          resume: () => events.push("older-pending"),
        });
      },
    });

    bounded.requestPermit((lease) => {
      bounded.track({
        dispose: () => undefined,
        resume: () => events.push("pump-current"),
      });
      lease.dispose();
    });

    expect(events).toEqual(["parent"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 3, pendingResources: 2 });

    bounded.track({ dispose: () => undefined, resume: () => events.push("external-current") });
    expect(events).toEqual(["parent", "older-pending", "pump-current", "external-current"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 4, pendingResources: 0 });
    bounded.retire();
  });

  it("snapshots lifecycle accessors once before later edges and contains callback failures", () => {
    const bounded = owner<string>();
    const reads = { dispose: 0, resume: 0, suspend: 0 };
    const hooks: string[] = [];
    const resource = Object.defineProperties(
      {},
      {
        dispose: {
          get: () => {
            reads.dispose += 1;
            if (reads.dispose > 1) throw new Error("dispose_getter_reread");
            return () => hooks.push("dispose");
          },
        },
        resume: {
          get: () => {
            reads.resume += 1;
            if (reads.resume > 1) throw new Error("resume_getter_reread");
            return () => hooks.push("resume");
          },
        },
        suspend: {
          get: () => {
            reads.suspend += 1;
            if (reads.suspend > 1) throw new Error("suspend_getter_reread");
            return () => hooks.push("suspend");
          },
        },
      },
    );

    expect(() => bounded.track(resource as never)).not.toThrow();
    expect(() => bounded.suspend()).not.toThrow();
    expect(() => bounded.resume()).not.toThrow();
    expect(() => bounded.retire()).not.toThrow();
    expect(reads).toEqual({ dispose: 1, resume: 1, suspend: 1 });
    expect(hooks).toEqual(["resume", "suspend", "resume", "dispose"]);
  });

  it("normalizes a throwing lifecycle getter without retaining a partial resource", () => {
    const bounded = owner<string>();
    const disposals: string[] = [];
    bounded.track({ dispose: () => disposals.push("older") });
    const hostile = Object.defineProperties(
      {},
      {
        dispose: {
          get: () => {
            throw new Error("hostile_dispose_getter");
          },
        },
        resume: { get: () => undefined },
        suspend: { get: () => undefined },
      },
    );

    expect(() => bounded.track(hostile as never)).toThrow(TypeError);
    expect(bounded.snapshot().ownedResources).toBe(1);
    expect(() => bounded.retire()).not.toThrow();
    expect(disposals).toEqual(["older"]);
  });

  it("rechecks terminal state after a lifecycle getter retires the owner", () => {
    const bounded = owner<string>();
    const disposals: string[] = [];
    bounded.track({ dispose: () => disposals.push("older") });
    let reads = 0;
    const reentrant = Object.defineProperties(
      {},
      {
        dispose: {
          get: () => {
            reads += 1;
            bounded.retire();
            return () => disposals.push("late");
          },
        },
        resume: { get: () => undefined },
        suspend: { get: () => undefined },
      },
    );

    expect(() => bounded.track(reentrant as never)).toThrow("bounded_owner_retired");
    expect(reads).toBe(1);
    expect(disposals).toEqual(["older"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 0, state: "retired" });
  });

  it("rejects recursive same-owner validation before reading nested callbacks", () => {
    const bounded = owner<string>({ maxItems: 1 });
    let nestedReads = 0;
    let nestedError: unknown;
    let outerDisposals = 0;
    const nested = new Proxy(
      {},
      {
        get: () => {
          nestedReads += 1;
          return () => undefined;
        },
      },
    );
    const outer = Object.defineProperty({}, "dispose", {
      get: () => {
        try {
          bounded.track(nested as never);
        } catch (error) {
          nestedError = error;
        }
        return () => {
          outerDisposals += 1;
        };
      },
    });

    const handle = bounded.track(outer as never);

    expect(nestedError).toMatchObject({ message: "bounded_owner_resource_reentrant" });
    expect(nestedReads).toBe(0);
    expect(bounded.snapshot().ownedResources).toBe(1);
    bounded.retire();
    handle.dispose();
    expect(outerDisposals).toBe(1);
  });

  it("clears the registration guard after an uncaught recursive getter failure", () => {
    const bounded = owner<string>({ maxItems: 1 });
    let nestedReads = 0;
    const nested = new Proxy(
      {},
      {
        get: () => {
          nestedReads += 1;
          return () => undefined;
        },
      },
    );
    const hostile = Object.defineProperty({}, "dispose", {
      get: () => {
        bounded.track(nested as never);
        return () => undefined;
      },
    });

    expect(() => bounded.track(hostile as never)).toThrow(TypeError);
    expect(nestedReads).toBe(0);
    expect(bounded.snapshot().ownedResources).toBe(0);
    const valid = bounded.track({ dispose: () => undefined });
    expect(bounded.snapshot().ownedResources).toBe(1);
    valid.dispose();
    bounded.retire();
  });

  it("invokes class lifecycle methods with their owning resource receiver", () => {
    class Resource {
      readonly #events: string[] = [];

      dispose(): void {
        this.#events.push("dispose");
      }

      events(): readonly string[] {
        return this.#events;
      }

      resume(): void {
        this.#events.push("resume");
      }

      suspend(): void {
        this.#events.push("suspend");
      }
    }

    const bounded = owner<string>();
    const resource = new Resource();
    bounded.track(resource);
    bounded.suspend();
    bounded.resume();
    bounded.retire();

    expect(resource.events()).toEqual(["resume", "suspend", "resume", "dispose"]);
  });

  it("allows an established lifecycle callback to register during hostile validation", () => {
    const bounded = owner<string>({ maxItems: 3 });
    const hooks: string[] = [];
    let childTracked = false;
    bounded.track({
      dispose: () => hooks.push("established:dispose"),
      resume: () => hooks.push("established:resume"),
      suspend: () => {
        hooks.push("established:suspend");
        bounded.track({
          dispose: () => hooks.push("child:dispose"),
          resume: () => hooks.push("child:resume"),
        });
        childTracked = true;
      },
    });
    hooks.length = 0;
    const hostile = Object.defineProperty({}, "dispose", {
      get: () => {
        bounded.suspend();
        return () => hooks.push("outer:dispose");
      },
    });

    expect(() => bounded.track(hostile as never)).not.toThrow();

    expect(childTracked).toBe(true);
    expect(hooks).toEqual(["established:suspend"]);
    expect(bounded.snapshot()).toMatchObject({
      ownedResources: 3,
      pendingResources: 2,
      state: "suspended",
    });
    bounded.resume();
    expect(hooks).toEqual(["established:suspend", "established:resume", "child:resume"]);
    expect(bounded.snapshot().pendingResources).toBe(0);
    bounded.retire();
    expect(hooks.slice(-3)).toEqual(["outer:dispose", "child:dispose", "established:dispose"]);
  });

  it("tracks once in the state selected by a reentrant lifecycle getter", () => {
    const bounded = owner<string>();
    const hooks: string[] = [];
    const reads = { dispose: 0, resume: 0, suspend: 0 };
    const reentrant = Object.defineProperties(
      {},
      {
        dispose: {
          get: () => {
            reads.dispose += 1;
            bounded.suspend();
            return () => hooks.push("dispose");
          },
        },
        resume: {
          get: () => {
            reads.resume += 1;
            return () => hooks.push("resume");
          },
        },
        suspend: {
          get: () => {
            reads.suspend += 1;
            return () => hooks.push("suspend");
          },
        },
      },
    );

    bounded.track(reentrant as never);

    expect(reads).toEqual({ dispose: 1, resume: 1, suspend: 1 });
    expect(hooks).toEqual([]);
    expect(bounded.snapshot().pendingResources).toBe(1);
    expect(bounded.snapshot().state).toBe("suspended");
    bounded.resume();
    expect(hooks).toEqual(["resume"]);
    expect(bounded.snapshot().pendingResources).toBe(0);
    bounded.retire();
    expect(hooks).toEqual(["resume", "dispose"]);
  });

  it("resumes resources tracked by a hook once after the stable edge snapshot", () => {
    const bounded = owner<string>();
    const hooks: string[] = [];
    let installLate = false;
    bounded.track({
      dispose: () => undefined,
      resume: () => {
        if (!installLate) return;
        hooks.push("first:start");
        installLate = false;
        bounded.track({
          dispose: () => undefined,
          resume: () => hooks.push("late"),
        });
        hooks.push("first:end");
      },
    });
    bounded.suspend();
    installLate = true;

    bounded.resume();

    expect(hooks).toEqual(["first:start", "first:end", "late"]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 2, state: "active" });
  });

  it("buffers reentrant enqueue but defers active work until resume completes", () => {
    const bounded = owner<string>();
    const events: string[] = [];
    const observed: { acquired?: BoundedLease | null; dequeued?: string | null } = {};
    let exercise = false;
    bounded.track({
      dispose: () => undefined,
      resume: () => {
        if (!exercise) return;
        events.push("resume:start");
        expect(bounded.enqueue("during-resume", 1)).toBe("accepted");
        observed.dequeued = bounded.dequeue();
        observed.acquired = bounded.acquire();
        bounded.requestPermit((lease) => {
          events.push("permit");
          lease.dispose();
        });
        observed.acquired?.dispose();
        events.push("resume:end");
      },
    });
    bounded.suspend();
    exercise = true;

    bounded.resume();

    expect(observed).toEqual({ acquired: null, dequeued: null });
    expect(events).toEqual(["resume:start", "resume:end", "permit"]);
    expect(bounded.dequeue()).toBe("during-resume");
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
  });

  it("admits only the stable waiter batch when callbacks replenish the queue", () => {
    const bounded = owner<string>();
    let admissions = 0;
    const replenish = (lease: BoundedLease): void => {
      admissions += 1;
      if (admissions < 8) bounded.requestPermit(replenish);
      lease.dispose();
    };

    bounded.requestPermit(replenish);

    expect(admissions).toBe(1);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 1 });
    bounded.retire();
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
  });

  it("advances one pre-existing waiter batch on each external admission entry", () => {
    const bounded = owner<string>({ maxActive: 1, maxItems: 1 });
    let admissions = 0;
    let replenish = true;
    let unexpectedNewAdmissions = 0;
    const prior = (lease: BoundedLease): void => {
      admissions += 1;
      if (replenish) bounded.requestPermit(prior);
      lease.dispose();
    };

    bounded.requestPermit(prior);
    expect(admissions).toBe(1);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 1 });

    const closed = bounded.requestPermit(() => {
      unexpectedNewAdmissions += 1;
    });
    expect(admissions).toBe(2);
    expect(closed.state()).toBe("items_exceeded");
    expect(unexpectedNewAdmissions).toBe(0);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 1 });

    expect(bounded.acquire()).toBeNull();
    expect(admissions).toBe(3);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 1 });

    replenish = false;
    const direct = bounded.acquire();
    expect(admissions).toBe(4);
    expect(direct).not.toBeNull();
    expect(bounded.snapshot()).toMatchObject({ active: 1, waitingPermits: 0 });
    direct?.dispose();
    bounded.retire();
  });

  it("evaluates a new permit request after the prior FIFO batch drains", () => {
    const bounded = owner<string>({ maxActive: 1, maxItems: 1 });
    let priorAdmissions = 0;
    let currentAdmissions = 0;
    const prior = (lease: BoundedLease): void => {
      priorAdmissions += 1;
      if (priorAdmissions === 1) bounded.requestPermit(prior);
      lease.dispose();
    };
    bounded.requestPermit(prior);

    const current = bounded.requestPermit((lease) => {
      currentAdmissions += 1;
      lease.dispose();
    });

    expect(priorAdmissions).toBe(2);
    expect(currentAdmissions).toBe(1);
    expect(current.state()).toBe("admitted");
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
    bounded.retire();
  });

  it("does not repump a replacement while evaluating the current request", () => {
    const bounded = owner<string>({ maxActive: 1, maxItems: 3 });
    let priorAdmissions = 0;
    let currentAdmissions = 0;
    const prior = (lease: BoundedLease): void => {
      priorAdmissions += 1;
      if (priorAdmissions < 4) bounded.requestPermit(prior);
      lease.dispose();
    };
    bounded.requestPermit(prior);

    const current = bounded.requestPermit((lease) => {
      currentAdmissions += 1;
      lease.dispose();
    });

    expect(priorAdmissions).toBe(2);
    expect(currentAdmissions).toBe(0);
    expect(current.state()).toBe("waiting");
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 2 });
    bounded.retire();
  });

  it("runs one stable deferred-resource batch per lifecycle edge", () => {
    const bounded = owner<string>();
    let callbacks = 0;
    let disposals = 0;
    let current: BoundedDisposable | undefined;
    const resource = (): Parameters<typeof bounded.track>[0] => ({
      dispose: () => {
        disposals += 1;
      },
      resume: () => {
        callbacks += 1;
        if (callbacks >= 8) return;
        current?.dispose();
        current = bounded.track(resource());
      },
    });
    bounded.suspend();
    current = bounded.track(resource());

    bounded.resume();

    expect(callbacks).toBe(2);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 1, state: "active" });
    bounded.retire();
    expect(disposals).toBe(3);
    expect(bounded.snapshot().ownedResources).toBe(0);
  });

  it("activates a deferred grandchild before its first suspend callback", () => {
    const bounded = owner<string>({ maxItems: 4 });
    const events: string[] = [];
    const disposals: string[] = [];
    let installChild = false;
    let installGrandchild = false;
    let grandchild: BoundedDisposable | undefined;
    bounded.track({
      dispose: () => disposals.push("parent"),
      resume: () => {
        if (!installChild) {
          events.push("parent:resume");
          return;
        }
        installChild = false;
        installGrandchild = true;
        events.push("parent:resume");
        bounded.track({
          dispose: () => disposals.push("child"),
          resume: () => {
            events.push("child:resume");
            if (!installGrandchild) return;
            installGrandchild = false;
            grandchild = bounded.track({
              dispose: () => disposals.push("grandchild"),
              resume: () => events.push("grandchild:resume"),
              suspend: () => events.push("grandchild:suspend"),
            });
          },
          suspend: () => events.push("child:suspend"),
        });
      },
      suspend: () => events.push("parent:suspend"),
    });
    events.length = 0;
    bounded.suspend();
    events.length = 0;
    installChild = true;

    bounded.resume();

    expect(events).toEqual(["parent:resume", "child:resume"]);
    const pendingSnapshot = bounded.snapshot();

    bounded.suspend();
    expect(events).toEqual(["parent:resume", "child:resume", "child:suspend", "parent:suspend"]);
    expect(pendingSnapshot).toMatchObject({ ownedResources: 3, pendingResources: 1 });

    bounded.resume();
    expect(events).toEqual([
      "parent:resume",
      "child:resume",
      "child:suspend",
      "parent:suspend",
      "parent:resume",
      "child:resume",
      "grandchild:resume",
    ]);
    expect(bounded.snapshot()).toMatchObject({ ownedResources: 3, pendingResources: 0 });

    bounded.retire();
    grandchild?.dispose();
    expect(disposals).toEqual(["grandchild", "child", "parent"]);
  });

  it("admits a legal hard-cap waiter batch without per-item queue scans", () => {
    const bounded = owner<string>({
      maxActive: HARD_MAX_ACTIVE_PERMITS,
      maxItems: HARD_MAX_RESOURCE_ITEMS,
    });
    let admissions = 0;
    bounded.suspend();
    for (let index = 0; index < HARD_MAX_RESOURCE_ITEMS; index += 1) {
      bounded.requestPermit(() => {
        admissions += 1;
      });
    }

    let failure: unknown;
    const indexScan = vi.spyOn(Array.prototype, "indexOf").mockImplementation(() => {
      throw new Error("waiter_batch_index_scan");
    });
    try {
      bounded.resume();
    } catch (error) {
      failure = error;
    } finally {
      indexScan.mockRestore();
    }
    const retirement = bounded.retire();

    expect(failure).toBeUndefined();
    expect(admissions).toBe(HARD_MAX_RESOURCE_ITEMS);
    expect(retirement.releasedPermits).toBe(HARD_MAX_ACTIVE_PERMITS);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
  });

  it("cancels a future waiter already extracted into the stable batch", () => {
    const bounded = owner<string>({ maxActive: 1, maxItems: 3 });
    const admissions: string[] = [];
    bounded.suspend();
    bounded.requestPermit((lease) => {
      admissions.push("first");
      second.dispose();
      lease.dispose();
    });
    const second = bounded.requestPermit((lease) => {
      admissions.push("second");
      lease.dispose();
    });
    bounded.requestPermit((lease) => {
      admissions.push("third");
      lease.dispose();
    });

    bounded.resume();

    expect(admissions).toEqual(["first", "third"]);
    expect(second.state()).toBe("canceled");
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: 0 });
    bounded.retire();
  });

  it("cancels future batch members and queues replacements without linear searches", () => {
    const size = 4_096;
    const half = size / 2;
    const bounded = owner<string>({ maxActive: 1, maxItems: size });
    const requests: ReturnType<typeof bounded.requestPermit>[] = [];
    const admissions: number[] = [];
    const replacements: number[] = [];
    bounded.suspend();
    for (let index = 0; index < size; index += 1) {
      requests.push(
        bounded.requestPermit((lease) => {
          admissions.push(index);
          if (index < half) {
            requests[index + half]?.dispose();
            bounded.requestPermit((replacement) => {
              replacements.push(index);
              replacement.dispose();
            });
          }
          lease.dispose();
        }),
      );
    }

    const indexScan = vi.spyOn(Array.prototype, "indexOf");
    bounded.resume();
    const linearSearches = indexScan.mock.calls.length;
    indexScan.mockRestore();

    expect(linearSearches).toBe(0);
    expect(admissions).toEqual(Array.from({ length: half }, (_value, index) => index));
    expect(requests.slice(half).every((request) => request.state() === "canceled")).toBe(true);
    expect(bounded.snapshot()).toMatchObject({ active: 0, waitingPermits: half });

    const direct = bounded.acquire();
    expect(replacements).toEqual(Array.from({ length: half }, (_value, index) => index));
    expect(direct).not.toBeNull();
    expect(bounded.snapshot()).toMatchObject({ active: 1, waitingPermits: 0 });
    direct?.dispose();
    bounded.retire();
  });

  it("contains reentrant permit callbacks and releases a thrown admission", () => {
    const bounded = owner<string>();
    const blocker = bounded.acquire();
    const order: string[] = [];
    bounded.requestPermit(() => {
      order.push("throws");
      throw new Error("feature callback failure");
    });
    const leases: { final?: BoundedLease } = {};
    bounded.requestPermit((lease) => {
      order.push("continues");
      leases.final = lease;
    });

    blocker?.dispose();
    expect(order).toEqual(["throws", "continues"]);
    expect(bounded.snapshot().active).toBe(1);
    leases.final?.dispose();
    expect(bounded.snapshot().active).toBe(0);
  });
});

describe("feature lifecycle resource labels", () => {
  it("keeps typed feature labels and bounded ownership outside both core entries", async () => {
    const featureKinds = [
      "upload",
      "stream",
      "poll",
    ] as const satisfies readonly FeatureResourceKind[];
    const resourceKinds: readonly ResourceKind[] = featureKinds;
    const bounded = owner<ResourceKind>();

    expect(resourceKinds).toEqual(["upload", "stream", "poll"]);
    expect(bounded.enqueue(featureKinds[0], 1)).toBe("accepted");
    expect(bounded.dequeue()).toBe("upload");

    for (const [entryPoint, format] of [
      ["src/entry-esm.ts", "esm"],
      ["src/entry-classic.ts", "iife"],
    ] as const) {
      const result = await build({
        absWorkingDir: BROWSER_ROOT,
        bundle: true,
        entryPoints: [entryPoint],
        format,
        metafile: true,
        minify: true,
        platform: "browser",
        treeShaking: true,
        write: false,
      });
      const inputs = Object.keys(result.metafile.inputs).map((name) => name.split("\\").join("/"));
      expect(inputs.some((name) => name.endsWith("/features/bounded.ts"))).toBe(false);
    }
  });
});
