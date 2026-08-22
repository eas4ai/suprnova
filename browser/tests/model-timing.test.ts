import { describe, expect, it } from "vitest";

import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import {
  ModelTimingCoordinator,
  parseModelTiming,
  type ModelTimingEvent,
} from "../src/models/timing.js";

class FakeTime implements RuntimeClock, RuntimeScheduler {
  #handle = 0;
  #now = 0;
  readonly #tasks = new Map<number, { readonly at: number; readonly callback: VoidFunction }>();

  now(): number {
    return this.#now;
  }

  microtask(callback: VoidFunction): void {
    callback();
  }

  animationFrame(callback: FrameRequestCallback): number {
    callback(this.#now);
    return 1;
  }

  cancelAnimationFrame(): void {
    return undefined;
  }

  timeout(callback: VoidFunction, milliseconds: number): number {
    this.#handle += 1;
    this.#tasks.set(this.#handle, { at: this.#now + milliseconds, callback });
    return this.#handle;
  }

  clearTimeout(handle: number): void {
    this.#tasks.delete(handle);
  }

  advance(milliseconds: number): void {
    this.#now += milliseconds;
    for (;;) {
      const due = [...this.#tasks.entries()]
        .filter(([, task]) => task.at <= this.#now)
        .sort((left, right) => left[1].at - right[1].at || left[0] - right[0])[0];
      if (due === undefined) return;
      this.#tasks.delete(due[0]);
      due[1].callback();
    }
  }
}

function update(
  timing: ModelTimingCoordinator,
  key: string,
  modifiers: readonly string[],
  event: ModelTimingEvent,
  callback: VoidFunction,
) {
  return timing.update(key, parseModelTiming(modifiers), event, callback);
}

describe("closed model timing policies", () => {
  it("parses the documented policies and rejects conflicting or malformed declarations", () => {
    expect(parseModelTiming([])).toEqual({ kind: "immediate" });
    expect(parseModelTiming(["change"])).toEqual({ kind: "change" });
    expect(parseModelTiming(["blur"])).toEqual({ kind: "blur" });
    expect(parseModelTiming(["action"])).toEqual({ kind: "action" });
    expect(parseModelTiming(["submit"])).toEqual({ kind: "submit" });
    expect(parseModelTiming(["debounce.250ms", "latest"])).toEqual({
      kind: "debounce",
      milliseconds: 250,
    });
    expect(parseModelTiming(["throttle.100ms", "serial"])).toEqual({
      kind: "throttle",
      milliseconds: 100,
    });
    expect(() => parseModelTiming(["blur", "debounce.100ms"])).toThrow("model_timing_conflict");
    expect(() => parseModelTiming(["debounce.999ms"])).toThrow("model_timing_invalid");
  });

  it("implements immediate, change, blur, action, and submit without conflating them", () => {
    const time = new FakeTime();
    const timing = new ModelTimingCoordinator(time, time);
    const calls: string[] = [];

    update(timing, "immediate", [], "input", () => calls.push("immediate"));
    update(timing, "change", ["change"], "input", () => calls.push("change"));
    update(timing, "blur", ["blur"], "input", () => calls.push("blur"));
    update(timing, "action", ["action"], "input", () => calls.push("action"));
    update(timing, "submit", ["submit"], "input", () => calls.push("submit"));
    expect(calls).toEqual(["immediate"]);

    update(timing, "change", ["change"], "change", () => calls.push("change-new"));
    update(timing, "blur", ["blur"], "blur", () => calls.push("blur-new"));
    expect(timing.flush("action")).toBe(true);
    expect(timing.flush("submit")).toBe(true);
    expect(calls).toEqual(["immediate", "change-new", "blur-new", "action", "submit"]);
  });

  it("debounces per full scope key and suppresses stale callbacks after flush", () => {
    const time = new FakeTime();
    const timing = new ModelTimingCoordinator(time, time);
    const calls: string[] = [];

    update(timing, "island-a:query:directive-1", ["debounce.100ms"], "input", () =>
      calls.push("old"),
    );
    time.advance(50);
    update(timing, "island-a:query:directive-1", ["debounce.100ms"], "input", () =>
      calls.push("new"),
    );
    update(timing, "island-b:query:directive-1", ["debounce.100ms"], "input", () =>
      calls.push("other-island"),
    );
    expect(timing.flush("island-a:query:directive-1")).toBe(true);
    expect(calls).toEqual(["new"]);
    time.advance(100);
    expect(calls).toEqual(["new", "other-island"]);
  });

  it("throttles with one leading and one newest trailing invocation", () => {
    const time = new FakeTime();
    const timing = new ModelTimingCoordinator(time, time);
    const calls: string[] = [];

    update(timing, "throttle", ["throttle.100ms"], "input", () => calls.push("first"));
    time.advance(20);
    update(timing, "throttle", ["throttle.100ms"], "input", () => calls.push("second"));
    time.advance(20);
    update(timing, "throttle", ["throttle.100ms"], "input", () => calls.push("newest"));
    expect(calls).toEqual(["first"]);
    time.advance(60);
    expect(calls).toEqual(["first", "newest"]);
  });

  it("cancels timers and makes every late callback ineligible after disposal", () => {
    const time = new FakeTime();
    const timing = new ModelTimingCoordinator(time, time);
    let calls = 0;
    update(timing, "debounce", ["debounce.100ms"], "input", () => {
      calls += 1;
    });
    timing.dispose();
    time.advance(100);
    expect(calls).toBe(0);
    expect(() => {
      update(timing, "late", [], "input", () => undefined);
    }).toThrow("model_timing_disposed");
  });

  it("does not consume timing capacity when an injected clock or timer port fails", () => {
    const time = new FakeTime();
    const badClock = new ModelTimingCoordinator({ now: () => Number.NaN }, time, 1);
    expect(() => update(badClock, "broken", ["throttle.100ms"], "input", () => undefined)).toThrow(
      "model_clock_invalid",
    );
    expect(() =>
      update(badClock, "replacement", ["action"], "input", () => undefined),
    ).not.toThrow();

    const badScheduler: RuntimeScheduler = {
      animationFrame: time.animationFrame.bind(time),
      cancelAnimationFrame: time.cancelAnimationFrame.bind(time),
      clearTimeout: time.clearTimeout.bind(time),
      microtask: time.microtask.bind(time),
      timeout() {
        throw new Error("host_timer_failed");
      },
    };
    const badTimer = new ModelTimingCoordinator(time, badScheduler, 1);
    expect(() => update(badTimer, "broken", ["debounce.100ms"], "input", () => undefined)).toThrow(
      "host_timer_failed",
    );
    expect(() =>
      update(badTimer, "replacement", ["submit"], "input", () => undefined),
    ).not.toThrow();

    const hostileClear: RuntimeScheduler = {
      animationFrame: time.animationFrame.bind(time),
      cancelAnimationFrame: time.cancelAnimationFrame.bind(time),
      clearTimeout() {
        throw new Error("host_clear_failed");
      },
      microtask: time.microtask.bind(time),
      timeout: time.timeout.bind(time),
    };
    const clearSafe = new ModelTimingCoordinator(time, hostileClear, 1);
    update(clearSafe, "pending", ["debounce.100ms"], "input", () => undefined);
    expect(() => {
      clearSafe.cancel("pending");
    }).not.toThrow();
    expect(() =>
      update(clearSafe, "replacement", ["action"], "input", () => undefined),
    ).not.toThrow();
  });
});
