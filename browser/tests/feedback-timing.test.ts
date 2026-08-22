import { describe, expect, it } from "vitest";

import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";
import { FeedbackTiming } from "../src/feedback/timing.js";

class FakeTime {
  #now = 0;
  #sequence = 0;
  readonly #timers = new Map<number, { readonly at: number; readonly callback: VoidFunction }>();

  readonly clock: RuntimeClock = { now: () => this.#now };
  readonly scheduler: RuntimeScheduler = {
    animationFrame: () => 1,
    cancelAnimationFrame: () => {
      // No animation frames are scheduled by this fixture.
    },
    clearTimeout: (handle) => {
      this.#timers.delete(handle);
    },
    microtask: (callback) => {
      callback();
    },
    timeout: (callback, delay) => {
      this.#sequence += 1;
      this.#timers.set(this.#sequence, { at: this.#now + delay, callback });
      return this.#sequence;
    },
  };

  advance(milliseconds: number): void {
    this.#now += milliseconds;
    for (;;) {
      const due = [...this.#timers.entries()]
        .filter(([, timer]) => timer.at <= this.#now)
        .sort(([left], [right]) => left - right)[0];
      if (due === undefined) return;
      this.#timers.delete(due[0]);
      due[1].callback();
    }
  }
}

describe("feedback visibility timing", () => {
  it("cancels a delayed state before it can flash", () => {
    const time = new FakeTime();
    const visibility: boolean[] = [];
    const timing = new FeedbackTiming(
      time.clock,
      time.scheduler,
      { delayMs: 150, minimumVisibleMs: 200, resetMs: null },
      (visible) => visibility.push(visible),
    );

    timing.update(true, "intent-1");
    time.advance(149);
    timing.update(false, null);
    time.advance(1_000);
    expect(visibility).toEqual([]);
    expect(timing.visible()).toBe(false);
  });

  it("honors minimum visibility without postponing authoritative settlement", () => {
    const time = new FakeTime();
    const visibility: boolean[] = [];
    const timing = new FeedbackTiming(
      time.clock,
      time.scheduler,
      { delayMs: 0, minimumVisibleMs: 200, resetMs: null },
      (visible) => visibility.push(visible),
    );

    timing.update(true, "intent-1");
    time.advance(50);
    timing.update(false, null);
    expect(timing.visible()).toBe(true);
    time.advance(149);
    expect(timing.visible()).toBe(true);
    time.advance(1);
    expect(timing.visible()).toBe(false);
    expect(visibility).toEqual([true, false]);
  });

  it("auto-resets one terminal transition and permits a later transition", () => {
    const time = new FakeTime();
    const visibility: boolean[] = [];
    const timing = new FeedbackTiming(
      time.clock,
      time.scheduler,
      { delayMs: 0, minimumVisibleMs: 100, resetMs: 500 },
      (visible) => visibility.push(visible),
    );

    timing.update(true, "intent-1");
    time.advance(500);
    expect(timing.visible()).toBe(false);
    timing.update(true, "intent-1");
    expect(timing.visible()).toBe(false);
    timing.update(true, "intent-2");
    expect(timing.visible()).toBe(true);
    timing.dispose();
    time.advance(1_000);
    expect(visibility).toEqual([true, false, true, false]);
  });
});
