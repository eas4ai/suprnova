import type { MorphIdentityEntry, MorphPlan } from "../morph/types.js";
import {
  MAX_TRANSITION_TARGETS,
  type TransitionCancelReason,
  type TransitionCompletion,
  type TransitionHandle,
  type TransitionKind,
  type TransitionRun,
  type TransitionSpec,
  type TransitionTarget,
} from "./types.js";
import { TransitionRunner } from "./runner.js";

const DIRECTIVE_PREFIX = "live:transition";
const DEFAULT_TRANSITION_MAXIMUM_MS = 2_000;

interface ParsedTransition {
  readonly name: string;
  readonly modifiers: ReadonlySet<string>;
}

interface PendingTransition {
  readonly entry: MorphIdentityEntry;
  readonly spec: TransitionSpec;
}

export interface MorphTransitions {
  readonly before: readonly TransitionTarget[];
  after(root: HTMLElement): readonly TransitionTarget[];
}

function parseTransition(element: Element | null): ParsedTransition | null {
  if (element === null) return null;
  for (const attribute of element.attributes) {
    if (attribute.name !== DIRECTIVE_PREFIX && !attribute.name.startsWith(`${DIRECTIVE_PREFIX}.`)) {
      continue;
    }
    const modifiers = new Set(attribute.name.split(".").slice(1));
    return Object.freeze({ modifiers, name: attribute.value });
  }
  return null;
}

function enabled(parsed: ParsedTransition, kind: TransitionKind): boolean {
  const modes = ["enter", "leave", "both"].filter((mode) => parsed.modifiers.has(mode));
  if (modes.length === 0 || parsed.modifiers.has("both")) return true;
  return parsed.modifiers.has(kind);
}

function spec(parsed: ParsedTransition, kind: TransitionKind): TransitionSpec {
  return Object.freeze({
    essential: parsed.modifiers.has("reduced-motion"),
    kind,
    maximumMs: DEFAULT_TRANSITION_MAXIMUM_MS,
    name: parsed.name,
  });
}

function target(element: Element, transition: TransitionSpec): TransitionTarget {
  return Object.freeze({ applyFinalState: () => undefined, element, spec: transition });
}

function changed(entry: MorphIdentityEntry): boolean {
  if (entry.current === null || entry.replacement === null) return true;
  try {
    return !entry.current.isEqualNode(entry.replacement);
  } catch {
    return true;
  }
}

function identityLabel(entry: MorphIdentityEntry): string {
  return entry.kind === "live_key"
    ? entry.value
    : entry.kind === "id"
      ? `#${entry.value}`
      : `island:${entry.value}`;
}

function resolve(root: HTMLElement, entry: MorphIdentityEntry): Element | null {
  if (entry.current?.isConnected === true) return entry.current;
  const candidates = [root, ...root.querySelectorAll("[data-suprnova-live-key], [id]")];
  for (const candidate of candidates) {
    if (
      entry.kind === "live_key" &&
      candidate.getAttribute("data-suprnova-live-key") === entry.value
    ) {
      return candidate;
    }
    if (entry.kind === "id" && candidate.id === entry.value) return candidate;
  }
  return null;
}

export function prepareMorphTransitions(plan: MorphPlan): MorphTransitions {
  const before: TransitionTarget[] = [];
  const after: PendingTransition[] = [];
  for (const entry of plan.identity.entries) {
    if (entry.kind === "nested_island") continue;
    let kind: TransitionKind;
    let parsed: ParsedTransition | null;
    if (entry.current === null) {
      kind = "enter";
      parsed = parseTransition(entry.replacement);
    } else if (entry.replacement === null) {
      kind = "leave";
      parsed = parseTransition(entry.current);
    } else if (plan.identity.moved.includes(identityLabel(entry))) {
      kind = "move";
      parsed = parseTransition(entry.replacement) ?? parseTransition(entry.current);
    } else if (changed(entry)) {
      kind = "state";
      parsed = parseTransition(entry.replacement) ?? parseTransition(entry.current);
    } else {
      continue;
    }
    if (parsed === null || !enabled(parsed, kind)) continue;
    const transition = spec(parsed, kind);
    if (kind === "leave") {
      if (entry.current === null) throw new Error("transition_identity_invalid");
      before.push(target(entry.current, transition));
    } else after.push(Object.freeze({ entry, spec: transition }));
    if (before.length + after.length > MAX_TRANSITION_TARGETS) {
      throw new Error("transition_target_limit");
    }
  }
  return Object.freeze({
    after: (root: HTMLElement) =>
      Object.freeze(
        after.flatMap((pending) => {
          const element = resolve(root, pending.entry);
          return element === null ? [] : [target(element, pending.spec)];
        }),
      ),
    before: Object.freeze(before),
  });
}

export class BrowserTransitionCompletion implements TransitionCompletion {
  start(element: Element, spec: TransitionSpec): TransitionHandle | null {
    const animationElement = element as Element & {
      getAnimations?: (options?: GetAnimationsOptions) => Animation[];
    };
    if (typeof animationElement.getAnimations !== "function") return null;
    let existing: ReadonlySet<Animation>;
    try {
      existing = new Set(animationElement.getAnimations({ subtree: false }));
    } catch {
      return null;
    }
    const classes = [
      "suprnova-live-transition",
      `suprnova-live-transition-${spec.kind}`,
      `suprnova-live-transition-${spec.name}`,
    ];
    element.classList.add(...classes);
    element.setAttribute("data-suprnova-live-transition-state", `${spec.kind}:${spec.name}`);
    try {
      void element.ownerDocument.defaultView?.getComputedStyle(element).animationName;
    } catch {
      // Style flushing is an optimization; missing style access falls through safely.
    }
    let animations: Animation[];
    try {
      animations = animationElement
        .getAnimations({ subtree: false })
        .filter((animation) => !existing.has(animation));
    } catch {
      this.#cleanup(element, classes);
      return null;
    }
    if (animations.length === 0) {
      this.#cleanup(element, classes);
      return null;
    }
    let cleaned = false;
    const cleanup = (): void => {
      if (cleaned) return;
      cleaned = true;
      this.#cleanup(element, classes);
    };
    let finished: Promise<void>;
    try {
      finished = Promise.all(animations.map((animation) => animation.finished)).then(
        () => {
          cleanup();
        },
        (error: unknown) => {
          cleanup();
          throw error;
        },
      );
    } catch (error: unknown) {
      cleanup();
      throw error;
    }
    return Object.freeze({
      cancel: () => {
        for (const animation of animations) {
          try {
            animation.cancel();
          } catch {
            // One broken animation cannot retain transition presentation.
          }
        }
        cleanup();
      },
      finished,
    });
  }

  #cleanup(element: Element, classes: readonly string[]): void {
    element.classList.remove(...classes);
    element.removeAttribute("data-suprnova-live-transition-state");
  }
}

export class TransitionLifecycle {
  readonly #runner: TransitionRunner;
  #active: TransitionRun | null = null;
  #disposed = false;

  constructor(runner: TransitionRunner) {
    this.#runner = runner;
  }

  begin(targets: readonly TransitionTarget[]): TransitionRun {
    if (this.#disposed) throw new Error("transition_lifecycle_disposed");
    this.#active?.cancel("superseded");
    const run = this.#runner.start(targets);
    this.#active = run;
    void run.finished.then(() => {
      if (this.#active === run) this.#active = null;
    });
    return run;
  }

  cancel(reason: TransitionCancelReason = "canceled"): void {
    const active = this.#active;
    this.#active = null;
    active?.cancel(reason);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.cancel("removed");
  }
}
