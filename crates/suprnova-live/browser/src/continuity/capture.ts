import type { MorphPlan } from "../morph/types.js";
import type { SignalContinuity } from "../signals/lifecycle.js";
import { captureFocus, captureSelections } from "./focus.js";
import { captureControls } from "./forms.js";
import { captureScroll } from "./scroll.js";
import { ContinuityError, type CompositionRecord, type ContinuityRecord } from "./types.js";

interface ActiveComposition {
  data: string;
  readonly element: Element;
}

function compositionTarget(event: Event): Element | null {
  return event.target instanceof Element ? event.target : null;
}

export class CompositionTracker {
  readonly #document: Document;
  #active: ActiveComposition | null = null;

  readonly #start = (event: Event): void => {
    const element = compositionTarget(event);
    if (element === null) return;
    this.#active = { data: (event as CompositionEvent).data, element };
  };

  readonly #update = (event: Event): void => {
    const element = compositionTarget(event);
    if (element === null || this.#active?.element !== element) return;
    this.#active.data = (event as CompositionEvent).data;
  };

  readonly #end = (event: Event): void => {
    if (this.#active?.element === compositionTarget(event)) this.#active = null;
  };

  constructor(document: Document) {
    this.#document = document;
    document.addEventListener("compositionstart", this.#start, true);
    document.addEventListener("compositionupdate", this.#update, true);
    document.addEventListener("compositionend", this.#end, true);
  }

  capture(plan: MorphPlan): CompositionRecord | null {
    const active = this.#active;
    if (active === null || !plan.currentRoot.contains(active.element)) return null;
    const entry = plan.identity.entries.find(({ current }) => current === active.element);
    if (entry === undefined) throw new ContinuityError("incompatible_state");
    if (entry.replacement === null) throw new ContinuityError("incompatible_state");
    return Object.freeze({ data: active.data, element: active.element, identity: entry.token });
  }

  dispose(): void {
    this.#document.removeEventListener("compositionstart", this.#start, true);
    this.#document.removeEventListener("compositionupdate", this.#update, true);
    this.#document.removeEventListener("compositionend", this.#end, true);
    this.#active = null;
  }
}

export interface ContinuityCaptureInput {
  readonly composition: CompositionTracker;
  readonly signalScopes: readonly SignalContinuity[];
}

export function captureContinuity(
  plan: MorphPlan,
  input: ContinuityCaptureInput,
): ContinuityRecord {
  const focus = captureFocus(plan);
  const controls = captureControls(plan);
  const selections = captureSelections(plan);
  const composition = input.composition.capture(plan);
  return Object.freeze({
    composition,
    controls,
    focusElement: focus.element,
    focusedKey: focus.focusedKey,
    focusVisible: focus.focusVisible,
    scroll: captureScroll(plan),
    selections,
    signalScopes: Object.freeze([...input.signalScopes]),
  });
}
