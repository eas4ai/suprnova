import type { ParsedDirective } from "./types.js";

export type DelegatedEventPhase = "capture" | "bubble";

export interface EventModifierTarget {
  hasAttribute(name: string): boolean;
  getAttribute(name: string): string | null;
}

export interface EventModifierDecision {
  readonly once: boolean;
  readonly prevent: boolean;
  readonly stop: boolean;
}

const KEY_FILTERS = Object.freeze(
  new Map<string, string>([
    ["enter", "Enter"],
    ["escape", "Escape"],
    ["space", " "],
    ["tab", "Tab"],
    ["up", "ArrowUp"],
    ["down", "ArrowDown"],
    ["left", "ArrowLeft"],
    ["right", "ArrowRight"],
  ]),
);

function disabled(target: EventModifierTarget): boolean {
  return target.hasAttribute("disabled") || target.getAttribute("aria-disabled") === "true";
}

export function evaluateEventModifiers(
  directive: ParsedDirective,
  event: Event,
  element: EventModifierTarget,
  origin: unknown,
  phase: DelegatedEventPhase,
): EventModifierDecision | null {
  const modifiers = directive.modifiers;
  if ((modifiers.includes("capture") ? "capture" : "bubble") !== phase) return null;
  if (disabled(element)) return null;
  if (modifiers.includes("trusted") && !event.isTrusted) return null;
  if (modifiers.includes("self") && origin !== element) return null;

  const keyFilters = modifiers.filter((modifier) => KEY_FILTERS.has(modifier));
  if (keyFilters.length > 1) return null;
  const keyFilter = keyFilters[0];
  if (keyFilter !== undefined) {
    if (!("key" in event) || event.key !== KEY_FILTERS.get(keyFilter)) return null;
  }

  return Object.freeze({
    once: modifiers.includes("once"),
    prevent: modifiers.includes("prevent"),
    stop: modifiers.includes("stop"),
  });
}

export function applyEventEffects(event: Event, decision: EventModifierDecision): void {
  if (decision.prevent) event.preventDefault();
  if (decision.stop) event.stopPropagation();
}
