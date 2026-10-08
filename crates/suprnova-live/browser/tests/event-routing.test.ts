import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
import {
  applyEventEffects,
  evaluateEventModifiers,
  type EventModifierTarget,
} from "../src/directives/modifiers.js";

function directive(name: string, value = "save") {
  const parsed = parseDirective(name, value);
  if (!parsed.ok) throw new Error(parsed.code);
  return parsed;
}

function target(disabled = false): EventModifierTarget {
  return {
    hasAttribute(name: string) {
      return disabled && name === "disabled";
    },
    getAttribute(name: string) {
      return disabled && name === "aria-disabled" ? "true" : null;
    },
  };
}

function event(overrides: Partial<Event> & { key?: string } = {}): Event {
  return {
    defaultPrevented: false,
    isTrusted: true,
    key: undefined,
    preventDefault() {
      Object.defineProperty(this, "defaultPrevented", { value: true });
    },
    stopPropagation() {
      Object.defineProperty(this, "cancelBubble", { value: true });
    },
    ...overrides,
  } as unknown as Event;
}

describe("event modifier routing", () => {
  it("filters phase, trust, self, disabled controls, and keyboard activation before effects", () => {
    const element = target();
    const capture = directive("live:click.capture.trusted.self");

    expect(
      evaluateEventModifiers(capture, event({ isTrusted: false }), element, element, "capture"),
    ).toBeNull();
    expect(evaluateEventModifiers(capture, event(), element, target(), "capture")).toBeNull();
    expect(evaluateEventModifiers(capture, event(), target(true), element, "capture")).toBeNull();
    expect(evaluateEventModifiers(capture, event(), element, element, "bubble")).toBeNull();

    const enter = directive("live:keydown.enter.prevent");
    expect(
      evaluateEventModifiers(enter, event({ key: "Escape" }), element, element, "bubble"),
    ).toBeNull();
    expect(
      evaluateEventModifiers(enter, event({ key: "Enter" }), element, element, "bubble"),
    ).toEqual({
      once: false,
      prevent: true,
      stop: false,
    });
  });

  it("preserves native behavior unless a validated routed directive requests effects", () => {
    let stopped = 0;
    const native = event({ stopPropagation: () => (stopped += 1) });
    const decision = evaluateEventModifiers(
      directive("live:click.prevent.stop.once"),
      native,
      target(),
      target(),
      "bubble",
    );
    expect(native.defaultPrevented).toBe(false);
    expect(stopped).toBe(0);
    expect(decision).not.toBeNull();

    if (decision === null) throw new Error("expected_modifier_decision");
    applyEventEffects(native, decision);
    expect(native.defaultPrevented).toBe(true);
    expect(stopped).toBe(1);
    expect(decision.once).toBe(true);
  });
});
