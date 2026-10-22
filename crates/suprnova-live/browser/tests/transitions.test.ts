import { describe, expect, it } from "vitest";

import { TransitionLifecycle } from "../src/transitions/lifecycle.js";
import { TransitionRunner } from "../src/transitions/runner.js";
import type {
  TransitionCompletion,
  TransitionHandle,
  TransitionSpec,
  TransitionTarget,
} from "../src/transitions/types.js";

function deferred(): {
  readonly promise: Promise<void>;
  readonly resolve: () => void;
  readonly reject: (error: unknown) => void;
} {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((accept, deny) => {
    resolve = accept;
    reject = deny;
  });
  return { promise, reject, resolve };
}

function spec(overrides: Partial<TransitionSpec> = {}): TransitionSpec {
  return Object.freeze({
    essential: false,
    kind: "state",
    maximumMs: 250,
    name: "fade",
    ...overrides,
  });
}

function target(finalStates: string[], overrides: Partial<TransitionSpec> = {}): TransitionTarget {
  return Object.freeze({
    applyFinalState: () => finalStates.push(overrides.kind ?? "state"),
    element: {} as Element,
    spec: spec(overrides),
  });
}

function harness(options: { readonly reducedMotion?: boolean; readonly supported?: boolean } = {}) {
  const completions: ReturnType<typeof deferred>[] = [];
  const canceled: number[] = [];
  const timers = new Map<number, VoidFunction>();
  let nextTimer = 1;
  const completion: TransitionCompletion = {
    start: () => {
      if (options.supported === false) return null;
      const pending = deferred();
      const index = completions.push(pending) - 1;
      return Object.freeze({
        cancel: () => canceled.push(index),
        finished: pending.promise,
      }) satisfies TransitionHandle;
    },
  };
  const runner = new TransitionRunner({
    completion,
    prefersReducedMotion: () => options.reducedMotion === true,
    scheduler: {
      clearTimeout: (handle) => timers.delete(handle),
      timeout: (callback) => {
        const handle = nextTimer;
        nextTimer += 1;
        timers.set(handle, callback);
        return handle;
      },
    },
  });
  return {
    canceled,
    completions,
    fireTimers: () => {
      for (const callback of [...timers.values()]) callback();
    },
    runner,
    timers,
  };
}

describe("bounded transition execution", () => {
  it.each(["enter", "leave", "move", "state"] as const)(
    "completes one %s transition and applies semantic final state once",
    async (kind) => {
      const finalStates: string[] = [];
      const test = harness();
      const run = test.runner.start([target(finalStates, { kind })]);
      test.completions[0]?.resolve();

      await expect(run.finished).resolves.toEqual([
        expect.objectContaining({ kind, status: "completed" }),
      ]);
      expect(finalStates).toEqual([kind]);
      expect(test.timers.size).toBe(0);
    },
  );

  it("cancels superseded work, ignores late completion, and applies both final states", async () => {
    const finalStates: string[] = [];
    const test = harness();
    const lifecycle = new TransitionLifecycle(test.runner);
    const first = lifecycle.begin([target(finalStates, { name: "first" })]);
    const second = lifecycle.begin([target(finalStates, { name: "second" })]);
    test.completions[0]?.resolve();
    test.completions[1]?.resolve();

    await expect(first.finished).resolves.toEqual([
      expect.objectContaining({ name: "first", status: "superseded" }),
    ]);
    await expect(second.finished).resolves.toEqual([
      expect.objectContaining({ name: "second", status: "completed" }),
    ]);
    expect(test.canceled).toEqual([0]);
    expect(finalStates).toEqual(["state", "state"]);
  });

  it("settles timeout and animation rejection without withholding final state", async () => {
    const timedFinal: string[] = [];
    const timed = harness();
    const timedRun = timed.runner.start([target(timedFinal)]);
    timed.fireTimers();
    await expect(timedRun.finished).resolves.toEqual([
      expect.objectContaining({ status: "timed_out" }),
    ]);
    expect(timed.canceled).toEqual([0]);
    expect(timedFinal).toEqual(["state"]);

    const rejectedFinal: string[] = [];
    const rejected = harness();
    const rejectedRun = rejected.runner.start([target(rejectedFinal)]);
    rejected.completions[0]?.reject(new Error("animation_failed"));
    await expect(rejectedRun.finished).resolves.toEqual([
      expect.objectContaining({ status: "failed" }),
    ]);
    expect(rejectedFinal).toEqual(["state"]);
  });

  it("skips nonessential reduced motion and unsupported completion ports", async () => {
    const reducedFinal: string[] = [];
    const reduced = harness({ reducedMotion: true });
    await expect(reduced.runner.start([target(reducedFinal)]).finished).resolves.toEqual([
      expect.objectContaining({ status: "reduced_motion" }),
    ]);
    expect(reduced.completions).toHaveLength(0);
    expect(reducedFinal).toEqual(["state"]);

    const unsupportedFinal: string[] = [];
    const unsupported = harness({ supported: false });
    await expect(unsupported.runner.start([target(unsupportedFinal)]).finished).resolves.toEqual([
      expect.objectContaining({ status: "unsupported" }),
    ]);
    expect(unsupportedFinal).toEqual(["state"]);
  });

  it("cancels active motion for removal and navigation without retaining timers", async () => {
    const finalStates: string[] = [];
    const test = harness();
    const lifecycle = new TransitionLifecycle(test.runner);
    const removal = lifecycle.begin([target(finalStates, { kind: "leave" })]);
    lifecycle.cancel("removed");
    await expect(removal.finished).resolves.toEqual([
      expect.objectContaining({ status: "removed" }),
    ]);

    const navigation = lifecycle.begin([target(finalStates, { kind: "move" })]);
    lifecycle.cancel("navigation");
    await expect(navigation.finished).resolves.toEqual([
      expect.objectContaining({ status: "navigation" }),
    ]);
    expect(test.timers.size).toBe(0);
    expect(finalStates).toEqual(["leave", "move"]);
  });

  it("validates the complete bounded batch before starting any animation", () => {
    const finalStates: string[] = [];
    const test = harness();
    expect(() =>
      test.runner.start([
        target(finalStates),
        target(finalStates, { maximumMs: 5_001, name: "invalid" }),
      ]),
    ).toThrow("transition_spec_invalid");
    expect(test.completions).toHaveLength(0);
    expect(test.timers.size).toBe(0);
    expect(finalStates).toEqual([]);
  });
});
