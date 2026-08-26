import { describe, expect, it, vi } from "vitest";

import type { JsonValue } from "../src/canonical.js";
import {
  PollTimer,
  resolvePollPolicy,
  type PollEnvironment,
  type PollPolicy,
} from "../src/async-updates/poll.js";
import { parseFeatureDirective } from "../src/features/directive-parser.js";
import { IslandRecord } from "../src/islands/record.js";
import { AsyncDocumentOwner } from "../src/async-updates/feature.js";
import type { AsyncRandomness, AsyncTimerPort } from "../src/async-updates/types.js";
import type {
  RuntimeFeatureDirectiveOwnership,
  AsyncRuntimeIslandPort,
} from "../src/features/contract.js";

class ControlledClock implements AsyncTimerPort {
  readonly pending = new Map<number, Readonly<{ callback: VoidFunction; due: number }>>();
  #next = 0;
  #now = 0;

  clearTimeout(handle: number): void {
    this.pending.delete(handle);
  }

  timeout(callback: VoidFunction, milliseconds: number): number {
    this.#next += 1;
    this.pending.set(this.#next, Object.freeze({ callback, due: this.#now + milliseconds }));
    return this.#next;
  }

  advance(milliseconds: number): void {
    const target = this.#now + milliseconds;
    for (;;) {
      const next = [...this.pending]
        .filter(([, timer]) => timer.due <= target)
        .sort((left, right) => left[1].due - right[1].due || left[0] - right[0])[0];
      if (next === undefined) break;
      this.pending.delete(next[0]);
      this.#now = next[1].due;
      next[1].callback();
    }
    this.#now = target;
  }

  advanceToNextTimer(): void {
    const due = Math.min(...[...this.pending.values()].map((timer) => timer.due));
    if (!Number.isFinite(due)) throw new Error("timer_not_found");
    this.advance(due - this.#now);
  }

  delays(): number[] {
    return [...this.pending.values()].map(({ due }) => due - this.#now);
  }
}

class Environment implements PollEnvironment {
  readonly #listeners = new Set<VoidFunction>();
  #online = true;
  #visible = true;

  isOnline(): boolean {
    return this.#online;
  }

  isVisible(): boolean {
    return this.#visible;
  }

  subscribe(listener: VoidFunction): VoidFunction {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  online(value: boolean): void {
    this.#online = value;
    for (const listener of this.#listeners) listener();
  }

  visible(value: boolean): void {
    this.#visible = value;
    for (const listener of this.#listeners) listener();
  }

  listenerCount(): number {
    return this.#listeners.size;
  }
}

function policy(overrides: Partial<PollPolicy> = {}): PollPolicy {
  return Object.freeze({
    initial: "wait",
    intervalMs: 10_000,
    jitterRatio: 0,
    mode: "poll_only",
    visibility: "visible",
    ...overrides,
  });
}

type TestFreshRenderCompletion = "succeeded" | "failed" | "canceled" | "retired";
type TestFreshRenderCallback = (completion: TestFreshRenderCompletion) => void;
type TestFreshRenderEnqueue = (
  reason: "poll",
  completion: TestFreshRenderCallback,
) => "queued" | "coalesced" | "retired";

function pollOwnership(
  root: Element,
  modifiers: readonly string[],
): RuntimeFeatureDirectiveOwnership {
  return Object.freeze({
    attributeName: `live:poll${modifiers.map((modifier) => `.${modifier}`).join("")}`,
    directive: Object.freeze({
      capability: "async@1" as const,
      modifiers: Object.freeze([...modifiers]),
      name: "poll",
      ok: true as const,
      role: null,
      value: "",
    }),
    element: root,
  });
}

function streamOwnership(
  root: Element,
  modifiers: readonly string[],
): RuntimeFeatureDirectiveOwnership {
  return Object.freeze({
    attributeName: `live:stream${modifiers.map((modifier) => `.${modifier}`).join("")}`,
    directive: Object.freeze({
      capability: "async@1" as const,
      modifiers: Object.freeze([...modifiers]),
      name: "stream",
      ok: true as const,
      role: null,
      value: "orders",
    }),
    element: root,
  });
}

function fixture(
  currentPolicy: PollPolicy = policy(),
  random = 0,
  enqueueFreshRender: TestFreshRenderEnqueue = vi.fn<TestFreshRenderEnqueue>(
    (_reason, completion) => {
      completion("succeeded");
      return "queued" as const;
    },
  ),
  observe: (state: string) => void = () => undefined,
) {
  const clock = new ControlledClock();
  const environment = new Environment();
  const randomness: AsyncRandomness = { number: () => random };
  const timer = new PollTimer({
    enqueueFreshRender,
    environment,
    policy: currentPolicy,
    randomness,
    timers: clock,
    observe,
  });
  return { clock, enqueueFreshRender, environment, timer };
}

describe("poll policy resolution", () => {
  const fallback = Object.freeze({
    initial: "wait" as const,
    intervalMs: 30_000,
    jitterRatio: 0.2,
    visibility: "visible" as const,
  });

  it("enforces an empty live:poll value from the generated directive contract", () => {
    expect(parseFeatureDirective("live:poll.visible.30s", "")).toMatchObject({
      name: "poll",
      ok: true,
      value: "",
    });
    expect(parseFeatureDirective("live:poll.visible.30s", "refresh")).toEqual({
      code: "invalid_value",
      fallback: "inert",
      ok: false,
    });
  });

  it("consumes every generated freshness combination without handwritten mode rules", () => {
    expect(resolvePollPolicy(null, null, null)).toBeNull();
    expect(resolvePollPolicy([], null, null)).toMatchObject({
      intervalMs: 30_000,
      mode: "poll_only",
    });
    expect(resolvePollPolicy(null, [], fallback)).toEqual({ ...fallback, mode: "hybrid" });
    expect(resolvePollPolicy(null, ["hybrid"], fallback)).toEqual({
      ...fallback,
      mode: "hybrid",
    });
    expect(resolvePollPolicy(["15s", "immediate", "always"], [], fallback)).toEqual({
      initial: "immediate",
      intervalMs: 15_000,
      jitterRatio: 0.2,
      mode: "hybrid",
      visibility: "always",
    });
    expect(resolvePollPolicy(null, ["push-only"], fallback)).toEqual({
      ...fallback,
      mode: "push_only",
    });
    expect(() => resolvePollPolicy([], ["push-only"], fallback)).toThrow("directive_conflict");
  });

  it("rejects missing or malformed signed hybrid fallback policy", () => {
    expect(() => resolvePollPolicy(null, [], null)).toThrow("poll_policy_invalid");
    expect(() => resolvePollPolicy(null, [], { ...fallback, intervalMs: 999 })).toThrow(
      "poll_policy_invalid",
    );
    expect(() => resolvePollPolicy(null, [], { ...fallback, intervalMs: 300_001 })).toThrow(
      "poll_policy_invalid",
    );
    expect(() => resolvePollPolicy(null, [], { ...fallback, jitterRatio: 1.01 })).toThrow(
      "poll_policy_invalid",
    );
  });
});

describe("controlled polling timer", () => {
  it("honors wait and immediate initial behavior and only enqueues a poll fresh render", () => {
    const waiting = fixture();
    waiting.timer.start();
    expect(waiting.enqueueFreshRender).not.toHaveBeenCalled();
    waiting.clock.advance(10_000);
    expect(waiting.enqueueFreshRender).toHaveBeenCalledWith("poll", expect.any(Function));

    const immediate = fixture(policy({ initial: "immediate" }));
    immediate.timer.start();
    expect(immediate.enqueueFreshRender).toHaveBeenCalledOnce();
    expect(immediate.enqueueFreshRender).toHaveBeenCalledWith("poll", expect.any(Function));
  });

  it("uses bounded positive jitter and rejects invalid randomness", () => {
    const jittered = fixture(policy({ intervalMs: 1_000, jitterRatio: 0.5 }), 0.5);
    jittered.timer.start();
    expect(jittered.clock.delays()).toEqual([1_250]);

    const invalid = fixture(policy(), 1);
    expect(() => {
      invalid.timer.start();
    }).toThrow("async_randomness_invalid");
  });

  it("pauses hidden or offline work until an eligibility event reschedules without catch-up", () => {
    const current = fixture();
    current.environment.visible(false);
    current.timer.start();
    expect(current.timer.status()).toBe("degraded");
    expect(current.clock.pending.size).toBe(0);
    current.clock.advance(60_000);
    expect(current.enqueueFreshRender).not.toHaveBeenCalled();

    current.environment.visible(true);
    expect(current.clock.pending.size).toBe(1);
    current.clock.advanceToNextTimer();
    expect(current.enqueueFreshRender).toHaveBeenCalledOnce();

    current.environment.online(false);
    expect(current.timer.status()).toBe("offline");
    expect(current.clock.pending.size).toBe(0);
    current.clock.advance(60_000);
    expect(current.enqueueFreshRender).toHaveBeenCalledOnce();

    current.environment.online(true);
    expect(current.enqueueFreshRender).toHaveBeenCalledOnce();
    expect(current.clock.pending.size).toBe(1);
    current.clock.advanceToNextTimer();
    expect(current.enqueueFreshRender).toHaveBeenCalledTimes(2);
  });

  it("derives initial offline status before observation and owns no ineligible timer", () => {
    const states: string[] = [];
    const current = fixture(policy(), 0, undefined, (state) => states.push(state));
    current.environment.online(false);

    current.timer.start();

    expect(current.timer.status()).toBe("offline");
    expect(states).toEqual(["offline"]);
    expect(current.clock.pending.size).toBe(0);
    current.clock.advance(300_000);
    expect(current.enqueueFreshRender).not.toHaveBeenCalled();
  });

  it("keeps offline presentation authoritative when an admitted refresh fails", () => {
    const completions: TestFreshRenderCallback[] = [];
    const enqueue = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completions.push(completion);
      return "queued";
    });
    const current = fixture(policy({ initial: "immediate" }), 0, enqueue);

    current.timer.start();
    expect(current.timer.status()).toBe("polling");
    current.environment.online(false);
    expect(current.timer.status()).toBe("offline");
    completions[0]?.("failed");

    expect(current.timer.status()).toBe("offline");
    expect(current.clock.pending.size).toBe(0);
  });

  it("continues while hidden only under the always policy", () => {
    const current = fixture(policy({ visibility: "always" }));
    current.environment.visible(false);
    current.timer.start();
    current.clock.advance(10_000);
    expect(current.enqueueFreshRender).toHaveBeenCalledOnce();
  });

  it("backs off from actual failed completion and resets only after actual success", () => {
    const completions: TestFreshRenderCallback[] = [];
    const enqueue = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completions.push(completion);
      return "queued";
    });
    const current = fixture(policy({ intervalMs: 1_000 }), 0.5, enqueue);
    current.timer.start();
    current.clock.advance(1_000);
    expect(current.timer.status()).toBe("polling");
    expect(current.clock.pending.size).toBe(0);
    completions[0]?.("failed");
    expect(current.timer.status()).toBe("degraded");
    expect(current.clock.delays()).toEqual([1_000]);
    current.clock.advance(1_000);
    expect(enqueue).toHaveBeenCalledTimes(2);
    completions[1]?.("succeeded");
    expect(current.timer.status()).toBe("current");
    expect(current.clock.delays()).toEqual([1_000]);
  });

  it("backs off canceled completion, ignores stale overlap, and closes on retired completion", () => {
    const completions: TestFreshRenderCallback[] = [];
    const enqueue = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completions.push(completion);
      return "queued";
    });
    const current = fixture(policy({ initial: "immediate", intervalMs: 1_000 }), 0.5, enqueue);
    current.timer.start();
    expect(enqueue).toHaveBeenCalledOnce();

    current.timer.suspend();
    completions[0]?.("failed");
    expect(current.timer.status()).toBe("suspended");
    current.timer.resume();
    current.clock.advanceToNextTimer();
    completions[1]?.("canceled");
    expect(current.timer.status()).toBe("degraded");
    current.clock.advanceToNextTimer();
    completions[2]?.("retired");
    expect(current.timer.status()).toBe("closed");
    expect(current.clock.pending.size).toBe(0);
  });

  it("caps repeated failure backoff and retires when the island scheduler is gone", () => {
    const failing = fixture(
      policy({ intervalMs: 300_000 }),
      0.999,
      vi.fn<TestFreshRenderEnqueue>(() => {
        throw new Error("network_failed");
      }),
    );
    failing.timer.start();
    failing.clock.advance(300_000);
    expect(failing.clock.delays()[0]).toBeLessThanOrEqual(300_000);

    const retired = fixture(
      policy({ intervalMs: 1_000 }),
      0,
      vi.fn<TestFreshRenderEnqueue>(() => "retired" as const),
    );
    retired.timer.start();
    retired.clock.advance(1_000);
    expect(retired.timer.status()).toBe("closed");
    expect(retired.clock.pending.size).toBe(0);
    expect(retired.environment.listenerCount()).toBe(0);
  });

  it("hybrid pauses only while continuity is proved and activates on loss", () => {
    const current = fixture(policy({ mode: "hybrid" }));
    current.timer.start();
    current.timer.continuity("current");
    current.clock.advance(60_000);
    expect(current.enqueueFreshRender).not.toHaveBeenCalled();
    expect(current.timer.status()).toBe("current");

    current.timer.continuity("degraded");
    expect(current.timer.status()).toBe("degraded");
    current.clock.advanceToNextTimer();
    expect(current.enqueueFreshRender).toHaveBeenCalledOnce();
  });

  it("push-only exposes degradation through one bounded observer but never invents fallback", () => {
    const states: string[] = [];
    const current = fixture(policy({ mode: "push_only" }), 0, undefined, (state) => {
      states.push(state);
      if (state === "degraded") throw new Error("observer_isolated");
    });
    current.timer.start();
    current.timer.continuity("degraded");
    current.clock.advance(300_000);
    expect(current.timer.status()).toBe("degraded");
    expect(current.clock.pending.size).toBe(0);
    expect(current.enqueueFreshRender).not.toHaveBeenCalled();
    expect(states).toEqual(["degraded"]);
    current.timer.dispose();
    expect(states).toEqual(["degraded", "closed"]);
  });

  it("suspends for bfcache, resumes once, and retires timers and listeners once", () => {
    const current = fixture();
    current.timer.start();
    expect(current.environment.listenerCount()).toBe(1);
    current.timer.suspend();
    current.clock.advance(60_000);
    expect(current.timer.status()).toBe("suspended");
    expect(current.enqueueFreshRender).not.toHaveBeenCalled();

    current.timer.resume();
    current.timer.resume();
    expect(current.clock.pending.size).toBe(1);
    current.timer.dispose();
    current.timer.dispose();
    expect(current.timer.status()).toBe("closed");
    expect(current.clock.pending.size).toBe(0);
    expect(current.environment.listenerCount()).toBe(0);
  });

  it("keeps pre-start suspended, offline, and retired lifecycle truth idempotent", () => {
    const suspendedStates: string[] = [];
    const suspended = fixture(policy({ intervalMs: 1_000 }), 0, undefined, (state) => {
      suspendedStates.push(state);
    });
    suspended.timer.suspend();
    suspended.timer.start();
    suspended.timer.start();
    expect(suspended.timer.status()).toBe("suspended");
    expect(suspendedStates).toEqual(["suspended"]);
    expect(suspended.clock.pending.size).toBe(0);
    expect(suspended.environment.listenerCount()).toBe(1);
    suspended.timer.resume();
    suspended.timer.resume();
    expect(suspended.timer.status()).toBe("degraded");
    expect(suspended.clock.pending.size).toBe(1);

    const offlineStates: string[] = [];
    const offline = fixture(policy({ intervalMs: 1_000 }), 0, undefined, (state) => {
      offlineStates.push(state);
    });
    offline.environment.online(false);
    offline.timer.start();
    offline.timer.start();
    expect(offline.timer.status()).toBe("offline");
    expect(offlineStates).toEqual(["offline"]);
    expect(offline.clock.pending.size).toBe(0);

    const retiredStates: string[] = [];
    const retired = fixture(policy({ intervalMs: 1_000 }), 0, undefined, (state) => {
      retiredStates.push(state);
    });
    retired.timer.dispose();
    retired.timer.start();
    retired.timer.resume();
    expect(retired.timer.status()).toBe("closed");
    expect(retiredStates).toEqual(["closed"]);
    expect(retired.environment.listenerCount()).toBe(0);

    suspended.timer.dispose();
    offline.timer.dispose();
  });

  it("spreads a 100-subscription recovery cohort without synchronized polling", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const timers = Array.from(
      { length: 100 },
      (_, index) =>
        new PollTimer({
          enqueueFreshRender: vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
            completion("succeeded");
            return "queued" as const;
          }),
          environment,
          policy: policy({ intervalMs: 5_000, jitterRatio: 0.2, mode: "hybrid" }),
          randomness: { number: () => (index + 0.5) / 100 },
          timers: clock,
        }),
    );
    for (const timer of timers) {
      timer.start();
      timer.continuity("current");
      timer.continuity("degraded");
    }

    const due = [...clock.pending.values()].map(({ due }) => due);
    expect(due).toHaveLength(100);
    expect(new Set(due).size).toBe(100);
    for (const timer of timers) timer.dispose();
  });

  it("keeps 100 offline and hidden islands event-driven at randomness boundaries", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const refreshes = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completion("succeeded");
      return "queued";
    });
    environment.online(false);
    environment.visible(false);
    const timers = Array.from(
      { length: 100 },
      (_, index) =>
        new PollTimer({
          enqueueFreshRender: refreshes,
          environment,
          policy: policy({ intervalMs: 5_000, jitterRatio: 0.2 }),
          randomness: { number: () => (index < 50 ? 0 : 0.999_999) },
          timers: clock,
        }),
    );

    for (const timer of timers) timer.start();
    expect(clock.pending.size).toBe(0);
    clock.advance(3_600_000);
    expect(refreshes).not.toHaveBeenCalled();

    environment.online(true);
    expect(clock.pending.size).toBe(0);
    environment.visible(true);
    expect(refreshes).not.toHaveBeenCalled();
    expect(clock.pending.size).toBe(100);
    const due = [...clock.pending.values()].map(({ due }) => due);
    expect(new Set(due).size).toBe(2);
    clock.advance(5_000);
    expect(refreshes).toHaveBeenCalledTimes(50);
    for (const timer of timers) timer.dispose();
  });
});

describe("fresh-render overlap remains owned by the island scheduler", () => {
  it("retains at most one in-flight plus one queued refresh and coalesces later ticks", () => {
    const element = { setAttribute: vi.fn() } as unknown as Element;
    const record = new IslandRecord(
      element,
      Object.freeze({
        component: "fixture.poll",
        documentKey: "document-poll",
        instanceId: "a".repeat(22),
        lazyComplete: true,
        protocolMinimum: 1,
        revision: 0n,
        runtimeContract: 1,
        slot: "poll-slot",
        snapshot: Object.freeze({}),
        snapshotForm: "instance",
      }),
    );

    const completions: TestFreshRenderCompletion[] = [];
    expect(record.enqueueFreshRender("poll", (result) => completions.push(result))).toBe("queued");
    const first = record.scheduler.ready()[0];
    if (first === undefined) throw new Error("missing_refresh_ticket");
    expect(record.scheduler.start(first)).toBe("accepted");
    expect(record.enqueueFreshRender("poll", (result) => completions.push(result))).toBe("queued");
    expect(record.enqueueFreshRender("poll", (result) => completions.push(result))).toBe(
      "coalesced",
    );
    expect(record.scheduler.snapshot()).toMatchObject({ inFlight: 1, queued: 1 });
    expect(completions).toEqual([]);

    expect(record.scheduler.settleTransport(first)).toBe("accepted");
    expect(record.scheduler.beginApplication(first)).toBe("accepted");
    expect(record.scheduler.finish(first, "rejected")).toBe("rejected");
    expect(completions).toEqual(["failed"]);
    const queued = record.scheduler.ready()[0];
    if (queued === undefined) throw new Error("missing_queued_refresh_ticket");
    expect(record.scheduler.start(queued)).toBe("accepted");
    expect(record.scheduler.settleTransport(queued)).toBe("accepted");
    expect(record.scheduler.beginApplication(queued)).toBe("accepted");
    expect(record.scheduler.finish(queued, "accepted")).toBe("accepted");
    expect(completions).toEqual(["failed", "succeeded", "succeeded"]);

    expect(record.enqueueFreshRender("poll", (result) => completions.push(result))).toBe("queued");
    const canceled = record.scheduler.ready()[0];
    if (canceled === undefined) throw new Error("missing_canceled_refresh_ticket");
    expect(record.scheduler.start(canceled)).toBe("accepted");
    expect(record.scheduler.cancel(canceled, { abortTransport: true })).toBe("canceled");
    expect(completions).toEqual(["failed", "succeeded", "succeeded", "canceled"]);

    expect(record.enqueueFreshRender("poll", (result) => completions.push(result))).toBe("queued");
    const retired = record.scheduler.ready()[0];
    if (retired === undefined) throw new Error("missing_retired_refresh_ticket");
    expect(record.scheduler.start(retired)).toBe("accepted");
    record.scheduler.retire();
    expect(completions).toEqual(["failed", "succeeded", "succeeded", "canceled", "retired"]);
  });
});

describe("poll-only feature integration", () => {
  it("is complete without stream authority or a transport adapter", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const root = Object.freeze({}) as Element;
    const refresh = vi.fn((_reason, completion: TestFreshRenderCallback) => {
      completion("succeeded");
      return "queued" as const;
    });
    const freshness: Readonly<{
      component: string;
      documentKey: string;
      slot: string;
      state: string;
    }>[] = [];
    const directive = Object.freeze({
      attributeName: "live:poll.5s",
      directive: Object.freeze({
        capability: "async@1" as const,
        modifiers: Object.freeze(["5s"]),
        name: "poll",
        ok: true as const,
        role: null,
        value: "",
      }),
      element: root,
    }) satisfies RuntimeFeatureDirectiveOwnership;
    const port = {
      consumeRegisteredEventCapability: vi.fn(),
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.poll",
        documentKey: "document-poll-only",
        slot: "poll-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => [directive],
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    } satisfies AsyncRuntimeIslandPort;
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        clock: { now: () => 0 },
        pollEnvironment: environment,
        randomness: { number: () => 0 },
        timers: clock,
        observeFreshness(observation) {
          expect(Object.isFrozen(observation)).toBe(true);
          freshness.push(observation);
        },
      },
    );

    const controller = owner.connectIsland(port);
    clock.advance(5_000);
    expect(refresh).toHaveBeenCalledOnce();
    expect(freshness.map(({ state }) => state)).toEqual(["degraded", "polling", "current"]);
    expect(freshness[freshness.length - 1]).toMatchObject({
      component: "fixture.poll",
      documentKey: "document-poll-only",
      slot: "poll-slot",
    });
    controller.dispose();
    expect(freshness[freshness.length - 1]?.state).toBe("closed");
    clock.advance(60_000);
    expect(refresh).toHaveBeenCalledOnce();
    owner.dispose();
  });

  it("rejects a non-callable public freshness observer", () => {
    expect(
      () =>
        new AsyncDocumentOwner(
          { diagnose: vi.fn(), onDispose: vi.fn() },
          {
            clock: { now: () => 0 },
            observeFreshness: "mutable" as never,
            randomness: { number: () => 0 },
            timers: new ControlledClock(),
          },
        ),
    ).toThrow("async_feature_configuration_invalid");
  });

  it("activates polling added by a committed morph on an initially directive-free island", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const root = Object.freeze({}) as Element;
    const refresh = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completion("succeeded");
      return "queued";
    });
    let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([]);
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        clock: { now: () => 0 },
        pollEnvironment: environment,
        randomness: { number: () => 0 },
        timers: clock,
      },
    );
    const controller = owner.connectIsland({
      consumeRegisteredEventCapability: vi.fn(),
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.poll",
        documentKey: "document-morph-add",
        slot: "poll-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    });

    controller.beforeMorph?.();
    ownerships = Object.freeze([pollOwnership(root, ["5s"])]);
    controller.afterMorph?.();
    clock.advance(5_000);

    expect(refresh).toHaveBeenCalledOnce();
    controller.dispose();
    owner.dispose();
  });

  it("keeps aborted morph policy inert and atomically replaces a committed interval", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const root = Object.freeze({}) as Element;
    const refresh = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completion("succeeded");
      return "queued";
    });
    let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([
      pollOwnership(root, ["5s"]),
    ]);
    const owner = new AsyncDocumentOwner(
      { diagnose: vi.fn(), onDispose: vi.fn() },
      {
        clock: { now: () => 0 },
        pollEnvironment: environment,
        randomness: { number: () => 0 },
        timers: clock,
      },
    );
    const controller = owner.connectIsland({
      consumeRegisteredEventCapability: vi.fn(),
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.poll",
        documentKey: "document-morph-interval",
        slot: "poll-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    });

    controller.beforeMorph?.();
    ownerships = Object.freeze([pollOwnership(root, ["10s"])]);
    controller.abortMorph?.();
    clock.advance(5_000);
    expect(refresh).toHaveBeenCalledOnce();

    controller.beforeMorph?.();
    controller.afterMorph?.();
    expect(clock.delays()).toEqual([10_000]);
    clock.advance(5_000);
    expect(refresh).toHaveBeenCalledOnce();
    clock.advance(5_000);
    expect(refresh).toHaveBeenCalledTimes(2);

    controller.beforeMorph?.();
    ownerships = Object.freeze([pollOwnership(root, ["immediate", "10s"])]);
    controller.afterMorph?.();
    expect(refresh).toHaveBeenCalledTimes(3);

    environment.visible(false);
    expect(clock.pending.size).toBe(0);
    controller.beforeMorph?.();
    ownerships = Object.freeze([pollOwnership(root, ["always", "10s"])]);
    controller.afterMorph?.();
    expect(clock.delays()).toEqual([10_000]);
    clock.advance(10_000);
    expect(refresh).toHaveBeenCalledTimes(4);
    controller.dispose();
    owner.dispose();
  });

  it("retires removed polling, fences late completion, and fails a morph conflict closed", () => {
    const clock = new ControlledClock();
    const environment = new Environment();
    const root = Object.freeze({}) as Element;
    const diagnose = vi.fn();
    const completions: TestFreshRenderCallback[] = [];
    const refresh = vi.fn<TestFreshRenderEnqueue>((_reason, completion) => {
      completions.push(completion);
      return "queued";
    });
    let ownerships: readonly RuntimeFeatureDirectiveOwnership[] = Object.freeze([
      pollOwnership(root, ["immediate", "5s"]),
    ]);
    const owner = new AsyncDocumentOwner(
      { diagnose, onDispose: vi.fn() },
      {
        clock: { now: () => 0 },
        pollEnvironment: environment,
        randomness: { number: () => 0 },
        timers: clock,
      },
    );
    const controller = owner.connectIsland({
      consumeRegisteredEventCapability: vi.fn(),
      dispatchRegisteredEvent: vi.fn(() => "dispatched" as const),
      element: root,
      enqueueFreshRender: refresh,
      identity: Object.freeze({
        component: "fixture.poll",
        documentKey: "document-morph-retire",
        slot: "poll-slot",
      }),
      onDispose: vi.fn(),
      queryDirectiveOwnership: () => ownerships,
      writePresentationSignal: vi.fn((_scope: string, _name: string, value: JsonValue) => value),
    });
    expect(refresh).toHaveBeenCalledOnce();

    controller.beforeMorph?.();
    ownerships = Object.freeze([]);
    controller.afterMorph?.();
    completions[0]?.("succeeded");
    clock.advance(60_000);
    expect(refresh).toHaveBeenCalledOnce();
    expect(clock.pending.size).toBe(0);

    controller.beforeMorph?.();
    ownerships = Object.freeze([streamOwnership(root, ["push-only"]), pollOwnership(root, ["5s"])]);
    controller.afterMorph?.();
    clock.advance(60_000);
    expect(refresh).toHaveBeenCalledOnce();
    expect(clock.pending.size).toBe(0);
    expect(diagnose).toHaveBeenCalledWith("operation_rejected");
    controller.dispose();
    owner.dispose();
  });
});
