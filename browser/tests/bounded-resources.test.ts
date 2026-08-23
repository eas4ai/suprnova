import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BoundedOwner,
  HARD_MAX_ACTIVE_PERMITS,
  HARD_MAX_RESOURCE_BYTES,
  HARD_MAX_RESOURCE_ITEMS,
  type BoundedLease,
} from "../src/features/bounded.js";
import type { FeatureResourceKind, ResourceKind } from "../src/lifecycle/resources.js";

const BROWSER_ROOT = fileURLToPath(new URL("../", import.meta.url));

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  expect(vi.getTimerCount()).toBe(0);
  vi.useRealTimers();
});

function owner<T>(overrides: Partial<ConstructorParameters<typeof BoundedOwner<T>>[0]> = {}) {
  return new BoundedOwner<T>({
    maxActive: 1,
    maxBytes: 8,
    maxItems: 2,
    ...overrides,
  });
}

describe("bounded feature owner", () => {
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
