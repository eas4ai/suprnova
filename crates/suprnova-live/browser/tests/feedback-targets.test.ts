import { describe, expect, it } from "vitest";

import { FeedbackTargetBinding, feedbackTimingPolicy } from "../src/feedback/targets.js";
import type { FeedbackSnapshot } from "../src/feedback/state.js";
import type { RuntimeClock, RuntimeScheduler } from "../src/runtime/ports.js";

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

class FakeElement {
  readonly #attributes = new Map<string, string>();
  readonly #classes = new Set<string>();
  readonly classList = {
    add: (name: string) => this.#classes.add(name),
    contains: (name: string) => this.#classes.has(name),
    remove: (name: string) => this.#classes.delete(name),
  };
  isConnected = true;
  textContent = "Original";

  constructor(
    readonly tagName: string,
    attributes: Readonly<Record<string, string>> = {},
  ) {
    for (const [name, value] of Object.entries(attributes)) this.#attributes.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.#attributes.has(name);
  }

  removeAttribute(name: string): void {
    this.#attributes.delete(name);
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }
}

function snapshot(state: FeedbackSnapshot["states"], intentId = "intent-1"): FeedbackSnapshot {
  return Object.freeze({ action: "save", field: null, intentId, recovery: "none", states: state });
}

describe("feedback directive targets", () => {
  it("delays loading, truthfully disables a native control, and restores its baseline", () => {
    const time = new FakeTime();
    const element = new FakeElement("BUTTON", { hidden: "" });
    const target = new FeedbackTargetBinding(
      element as unknown as Element,
      "loading",
      ["show", "disabled", "busy", "class"],
      "action:save",
      time.clock,
      time.scheduler,
    );

    target.update(snapshot(new Set(["loading"])), "intent-1");
    time.advance(feedbackTimingPolicy("loading").delayMs - 1);
    expect(element.hasAttribute("hidden")).toBe(true);
    expect(element.hasAttribute("disabled")).toBe(false);

    time.advance(1);
    expect(element.hasAttribute("hidden")).toBe(false);
    expect(element.hasAttribute("disabled")).toBe(true);
    expect(element.getAttribute("aria-busy")).toBe("true");
    expect(element.classList.contains("live-loading")).toBe(true);

    target.update(snapshot(new Set(["idle"])), null);
    time.advance(feedbackTimingPolicy("loading").minimumVisibleMs);
    expect(element.hasAttribute("hidden")).toBe(true);
    expect(element.hasAttribute("disabled")).toBe(false);
    expect(element.hasAttribute("aria-busy")).toBe(false);
    expect(element.classList.contains("live-loading")).toBe(false);
  });

  it("never disables links and restores live-region content on disposal", () => {
    const time = new FakeTime();
    const element = new FakeElement("A", { href: "/escape" });
    const target = new FeedbackTargetBinding(
      element as unknown as Element,
      "retrying",
      ["disabled", "live.polite"],
      "action:save",
      time.clock,
      time.scheduler,
    );

    target.update(snapshot(new Set(["retrying"])), "intent-1", "retry");
    time.advance(feedbackTimingPolicy("retrying").delayMs);
    expect(element.hasAttribute("disabled")).toBe(false);
    expect(element.getAttribute("aria-live")).toBe("polite");
    expect(element.textContent).toBe("Retrying");

    target.update(snapshot(new Set(["retrying"])), "intent-1", "retry");
    expect(element.textContent).toBe("Retrying");
    target.dispose();
    expect(element.textContent).toBe("Original");
    expect(element.hasAttribute("aria-live")).toBe(false);
  });

  it("cancels pending presentation when a target is removed", () => {
    const time = new FakeTime();
    const element = new FakeElement("DIV", { hidden: "" });
    const target = new FeedbackTargetBinding(
      element as unknown as Element,
      "loading",
      ["show"],
      "island:primary",
      time.clock,
      time.scheduler,
    );
    target.update(snapshot(new Set(["loading"])), "intent-1");
    element.isConnected = false;
    target.dispose();
    time.advance(1_000);
    expect(element.hasAttribute("hidden")).toBe(true);
  });
});
