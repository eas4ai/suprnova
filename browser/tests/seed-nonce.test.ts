import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
import { LazyCoordinator, LazyIntentMarker } from "../src/islands/lazy.js";
import { PROMOTION_NONCE_BYTES } from "../src/islands/nonce.js";
import type { IslandRecord } from "../src/islands/record.js";
import { createServerIntent, type IntentSource } from "../src/scheduler/intent.js";
import type { RuntimeObserverFactory, RuntimeRandomness } from "../src/runtime/ports.js";

const source = {
  directive: { ok: true },
  element: {},
  eventType: "click",
  island: {},
  trusted: true,
} as unknown as IntentSource;

describe("seed intent nonce ownership", () => {
  it("uses at least 128 random bits once and keeps them only for compatible retries", () => {
    const requests: number[] = [];
    let fill = 0;
    const randomness: RuntimeRandomness = {
      randomBytes(length: number) {
        requests.push(length);
        fill += 1;
        return new Uint8Array(length).fill(fill);
      },
    };

    const first = createServerIntent(
      source,
      [{ kind: "invoke_action", name: "save", arguments: {} }],
      randomness,
      true,
    );
    const nonce = first.promotionNonce();
    expect(requests).toEqual([PROMOTION_NONCE_BYTES]);
    expect(PROMOTION_NONCE_BYTES).toBeGreaterThanOrEqual(16);
    expect(first.promotionNonce()).toBe(nonce);
    expect(Object.isFrozen(first)).toBe(true);
    expect(Object.isFrozen(first.operations)).toBe(true);
    expect(Object.isFrozen(first.operations[0])).toBe(true);
    expect(Object.isFrozen(first.operations[0]?.kind === "invoke_action" ? first.operations[0].arguments : null)).toBe(true);
    first.finish("accepted");
    expect(first.promotionNonce()).toBeNull();

    const second = createServerIntent(
      source,
      [{ kind: "invoke_action", name: "save", arguments: {} }],
      randomness,
      true,
    );
    expect(second.promotionNonce()).not.toBe(nonce);
  });

  it("fails closed when cryptographic randomness is unavailable", () => {
    const randomness: RuntimeRandomness = {
      randomBytes() {
        throw new Error("crypto_unavailable");
      },
    };
    expect(() =>
      createServerIntent(
        source,
        [{ kind: "invoke_action", name: "save", arguments: {} }],
        randomness,
        true,
      ),
    ).toThrow("promotion_nonce_unavailable");
  });

  it("erases nonce references for every terminal intent disposition", () => {
    const randomness: RuntimeRandomness = {
      randomBytes: (length) => new Uint8Array(length).fill(9),
    };
    for (const finish of ["terminal", "canceled", "exhausted", "rejected"] as const) {
      const intent = createServerIntent(
        source,
        [{ kind: "invoke_action", name: "save", arguments: {} }],
        randomness,
        true,
      );
      expect(intent.promotionNonce()).not.toBeNull();
      intent.finish(finish);
      expect(intent.promotionNonce()).toBeNull();
    }
  });
});

describe("lazy intent identity", () => {
  it("queues once and cannot be revived after resolution or retirement", () => {
    const marker = new LazyIntentMarker();
    expect(marker.queue()).toBe(true);
    expect(marker.queue()).toBe(false);
    marker.resolve();
    expect(marker.queue()).toBe(false);

    const retired = new LazyIntentMarker();
    retired.retire();
    expect(retired.queue()).toBe(false);
  });

  it("deduplicates repeated observer activation for one surviving island identity", () => {
    let intersectionCallback: IntersectionObserverCallback | undefined;
    const observer = {
      disconnect() {
        return undefined;
      },
      observe() {
        return undefined;
      },
      takeRecords: () => [],
      unobserve() {
        return undefined;
      },
      root: null,
      rootMargin: "0px",
      thresholds: [0],
    } as unknown as IntersectionObserver;
    const observers: RuntimeObserverFactory = {
      intersection(callback) {
        intersectionCallback = callback;
        return observer;
      },
      mutation() {
        throw new Error("unused_mutation_observer");
      },
    };
    const randomness: RuntimeRandomness = {
      randomBytes: (length) => new Uint8Array(length).fill(3),
    };
    const disposers: VoidFunction[] = [];
    let queued = 0;
    const record = {
      active: () => true,
      enqueue: () => {
        queued += 1;
        return true;
      },
      metadata: { documentKey: "lazy", lazyComplete: false, snapshotForm: "instance" },
      onDispose: (disposer: VoidFunction) => {
        disposers.push(disposer);
      },
    } as unknown as IslandRecord;
    const parsed = parseDirective("live:lazy.visible", "");
    if (!parsed.ok) throw new Error(parsed.code);
    const element = {} as Element;
    const coordinator = new LazyCoordinator(observers, randomness);
    coordinator.connect(record, [{ directive: parsed, element }]);
    if (intersectionCallback === undefined) throw new Error("missing_intersection_callback");
    const entry = { isIntersecting: true, target: element } as IntersectionObserverEntry;
    intersectionCallback([entry], observer);
    intersectionCallback([entry], observer);
    coordinator.connect(record, [{ directive: parsed, element }]);
    expect(queued).toBe(1);
    for (const dispose of disposers) dispose();
  });
});
