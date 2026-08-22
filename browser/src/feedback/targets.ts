import type { OwnedDirective } from "../directives/ownership.js";
import type { RecoveryState } from "../application/recovery.js";
import type { IslandRecord } from "../islands/record.js";
import type { ModelState } from "../models/state.js";
import type { RuntimeClock, RuntimeScheduler } from "../runtime/ports.js";
import {
  FeedbackAnnouncer,
  type FeedbackAnnouncementKind,
  type FeedbackPoliteness,
} from "./announcer.js";
import {
  projectFeedback,
  type FeedbackScope,
  type FeedbackSnapshot,
  type FeedbackState,
} from "./state.js";
import { FeedbackTiming, type FeedbackTimingPolicy } from "./timing.js";

const FEEDBACK_STATES: ReadonlySet<string> = new Set([
  "idle",
  "dirty",
  "queued",
  "loading",
  "validating",
  "success",
  "interrupted",
  "offline",
  "retrying",
  "error",
]);
const DISABLEABLE_TAGS: ReadonlySet<string> = new Set([
  "BUTTON",
  "FIELDSET",
  "INPUT",
  "OPTGROUP",
  "OPTION",
  "SELECT",
  "TEXTAREA",
]);

const IMMEDIATE = Object.freeze({ delayMs: 0, minimumVisibleMs: 0, resetMs: null });
const LOADING = Object.freeze({ delayMs: 150, minimumVisibleMs: 200, resetMs: null });
const RETRYING = Object.freeze({ delayMs: 0, minimumVisibleMs: 200, resetMs: null });
const TERMINAL = Object.freeze({ delayMs: 0, minimumVisibleMs: 100, resetMs: 2_000 });

export function feedbackTimingPolicy(state: FeedbackState): FeedbackTimingPolicy {
  if (state === "loading" || state === "validating") return LOADING;
  if (state === "retrying") return RETRYING;
  if (state === "success" || state === "interrupted" || state === "error") return TERMINAL;
  return IMMEDIATE;
}

function restoreAttribute(element: Element, name: string, value: string | null): void {
  if (value === null) element.removeAttribute(name);
  else element.setAttribute(name, value);
}

function defaultAnnouncement(state: FeedbackState): FeedbackAnnouncementKind | null {
  switch (state) {
    case "idle":
    case "dirty":
    case "queued":
    case "loading":
    case "validating":
    case "offline":
    case "success":
      return state;
    case "interrupted":
      return "interruption";
    case "retrying":
      return "retry";
    case "error":
      return "failure";
  }
}

interface TargetPresentation {
  readonly state: FeedbackState;
  readonly modifiers: ReadonlySet<string>;
  readonly scope: string;
  visible: boolean;
  transition: string | null;
  announcementKind: FeedbackAnnouncementKind | null;
  politeness: FeedbackPoliteness | null;
}

class FeedbackElementPresentation {
  readonly #element: Element;
  readonly #scheduler: RuntimeScheduler;
  readonly #announcer: FeedbackAnnouncer;
  readonly #entries = new Map<symbol, TargetPresentation>();
  readonly #baselineClasses = new Map<string, boolean>();
  readonly #baselineHidden: boolean;
  readonly #baselineDisabled: boolean;
  readonly #baselineBusy: string | null;
  readonly #baselineLive: string | null;
  readonly #baselineRole: string | null;
  readonly #baselineText: string | null;
  #announcementGeneration = 0;

  constructor(element: Element, scheduler: RuntimeScheduler) {
    this.#element = element;
    this.#scheduler = scheduler;
    this.#baselineHidden = element.hasAttribute("hidden");
    this.#baselineDisabled = element.hasAttribute("disabled");
    this.#baselineBusy = element.getAttribute("aria-busy");
    this.#baselineLive = element.getAttribute("aria-live");
    this.#baselineRole = element.getAttribute("role");
    this.#baselineText = element.textContent;
    this.#announcer = new FeedbackAnnouncer((announcement) => {
      this.#announcementGeneration += 1;
      const generation = this.#announcementGeneration;
      this.#element.textContent = "";
      this.#scheduler.microtask(() => {
        if (generation === this.#announcementGeneration && this.#entries.size > 0) {
          this.#element.textContent = announcement.message;
        }
      });
    });
  }

  register(state: FeedbackState, modifiers: readonly string[], scope: string): symbol {
    const token = Symbol(state);
    const className = `live-${state}`;
    if (!this.#baselineClasses.has(className)) {
      this.#baselineClasses.set(className, this.#element.classList.contains(className));
    }
    this.#entries.set(token, {
      announcementKind: null,
      modifiers: new Set(modifiers),
      politeness: null,
      scope,
      state,
      transition: null,
      visible: false,
    });
    return token;
  }

  configure(
    token: symbol,
    transition: string | null,
    announcementKind: FeedbackAnnouncementKind | null,
  ): void {
    const entry = this.#entries.get(token);
    if (entry === undefined) return;
    entry.transition = transition;
    entry.announcementKind = announcementKind;
    entry.politeness = entry.modifiers.has("live.assertive")
      ? "assertive"
      : entry.modifiers.has("live.polite")
        ? "polite"
        : null;
  }

  present(token: symbol, visible: boolean, announce: boolean): void {
    const entry = this.#entries.get(token);
    if (entry === undefined) return;
    entry.visible = visible;
    this.#recompute();
    if (
      announce &&
      visible &&
      entry.politeness !== null &&
      entry.transition !== null &&
      entry.announcementKind !== null
    ) {
      this.#element.setAttribute("aria-live", entry.politeness);
      this.#element.setAttribute("role", entry.politeness === "assertive" ? "alert" : "status");
      this.#announcer.announce(
        entry.scope,
        entry.announcementKind,
        entry.transition,
        entry.politeness,
      );
    }
  }

  unregister(token: symbol): void {
    if (!this.#entries.delete(token)) return;
    if (this.#entries.size === 0) this.#restore();
    else this.#recompute();
  }

  empty(): boolean {
    return this.#entries.size === 0;
  }

  #recompute(): void {
    const entries = [...this.#entries.values()];
    const visible = entries.filter((entry) => entry.visible);
    const hides = visible.some((entry) => entry.modifiers.has("hide"));
    const shows = visible.some(
      (entry) => entry.modifiers.size === 0 || entry.modifiers.has("show"),
    );
    this.#setBooleanAttribute("hidden", hides || (!shows && this.#baselineHidden));
    for (const [className, baseline] of this.#baselineClasses) {
      const state = className.slice("live-".length);
      const active = visible.some((entry) => entry.state === state && entry.modifiers.has("class"));
      if (active || baseline) this.#element.classList.add(className);
      else this.#element.classList.remove(className);
    }
    if (DISABLEABLE_TAGS.has(this.#element.tagName)) {
      const disabled = visible.some((entry) => entry.modifiers.has("disabled"));
      this.#setBooleanAttribute("disabled", disabled || this.#baselineDisabled);
    }
    const busy = visible.some((entry) => entry.modifiers.has("busy"));
    if (busy) this.#element.setAttribute("aria-busy", "true");
    else restoreAttribute(this.#element, "aria-busy", this.#baselineBusy);
    const liveEntries = visible.filter((entry) => entry.politeness !== null);
    const live = liveEntries[liveEntries.length - 1];
    const politeness = live?.politeness ?? null;
    if (politeness === null) this.#restoreLiveRegion();
    else {
      this.#element.setAttribute("aria-live", politeness);
      this.#element.setAttribute("role", politeness === "assertive" ? "alert" : "status");
    }
  }

  #setBooleanAttribute(name: string, present: boolean): void {
    if (present) this.#element.setAttribute(name, "");
    else this.#element.removeAttribute(name);
  }

  #restoreLiveRegion(): void {
    this.#announcementGeneration += 1;
    restoreAttribute(this.#element, "aria-live", this.#baselineLive);
    restoreAttribute(this.#element, "role", this.#baselineRole);
    this.#element.textContent = this.#baselineText;
  }

  #restore(): void {
    this.#setBooleanAttribute("hidden", this.#baselineHidden);
    for (const [className, baseline] of this.#baselineClasses) {
      if (baseline) this.#element.classList.add(className);
      else this.#element.classList.remove(className);
    }
    if (DISABLEABLE_TAGS.has(this.#element.tagName)) {
      this.#setBooleanAttribute("disabled", this.#baselineDisabled);
    }
    restoreAttribute(this.#element, "aria-busy", this.#baselineBusy);
    this.#restoreLiveRegion();
  }
}

export class FeedbackTargetBinding {
  readonly #element: Element;
  readonly #state: FeedbackState;
  readonly #presentation: FeedbackElementPresentation;
  readonly #token: symbol;
  readonly #timing: FeedbackTiming;
  #transition: string | null = null;
  #disposed = false;

  constructor(
    element: Element,
    state: FeedbackState,
    modifiers: readonly string[],
    scope: string,
    clock: RuntimeClock,
    scheduler: RuntimeScheduler,
    presentation = new FeedbackElementPresentation(element, scheduler),
  ) {
    this.#element = element;
    this.#state = state;
    this.#presentation = presentation;
    this.#token = presentation.register(state, modifiers, scope);
    this.#timing = new FeedbackTiming(clock, scheduler, feedbackTimingPolicy(state), (visible) => {
      this.#present(visible);
    });
  }

  update(
    snapshot: FeedbackSnapshot,
    transition: string | null,
    announcementKind: FeedbackAnnouncementKind | null = null,
  ): void {
    if (this.#disposed) return;
    const changed = transition !== this.#transition;
    this.#transition = transition;
    this.#presentation.configure(
      this.#token,
      transition,
      announcementKind ?? defaultAnnouncement(this.#state),
    );
    this.#timing.update(snapshot.states.has(this.#state), transition);
    if (changed && this.#timing.visible()) this.#present(true);
  }

  element(): Element {
    return this.#element;
  }

  presentationEmpty(): boolean {
    return this.#presentation.empty();
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#timing.dispose();
    this.#disposed = true;
    this.#presentation.unregister(this.#token);
  }

  #present(visible: boolean): void {
    this.#presentation.present(this.#token, visible, visible);
  }
}

interface RecordFeedback {
  readonly bindings: Set<ManagedBinding>;
  readonly model: ModelState | null;
  readonly presentations: Map<Element, FeedbackElementPresentation>;
  readonly unsubscribeModel: VoidFunction | null;
  readonly unsubscribeScheduler: VoidFunction;
  recovery: RecoveryState;
}

interface ManagedBinding {
  readonly attributeName: string;
  readonly binding: FeedbackTargetBinding;
  readonly scope: FeedbackScope;
  readonly state: FeedbackState;
}

function scopeFor(record: IslandRecord, model: ModelState | null, target: string): FeedbackScope {
  if (model?.fields().includes(target) === true)
    return Object.freeze({ kind: "field", value: target });
  if (target === record.metadata.documentKey || target === record.metadata.slot) {
    return Object.freeze({ kind: "island", value: record.metadata.documentKey });
  }
  return Object.freeze({ kind: "action", value: target });
}

function transitionFor(snapshot: FeedbackSnapshot, scope: FeedbackScope, model: ModelState | null) {
  if (snapshot.intentId !== null) return snapshot.intentId;
  if (scope.kind === "field" && model !== null) {
    const field = model.snapshot(scope.value);
    return `model.${String(field.editSequence)}.${String(field.validation.length)}`;
  }
  return `state.${scope.kind}.${scope.value}`;
}

function announcementFor(
  state: FeedbackState,
  scope: FeedbackScope,
  model: ModelState | null,
): FeedbackAnnouncementKind | null {
  if (
    state === "error" &&
    scope.kind === "field" &&
    model !== null &&
    model.snapshot(scope.value).validation.length > 0
  ) {
    return "validation";
  }
  return defaultAnnouncement(state);
}

function within(node: Node, element: Element): boolean {
  let current: Node | null = element;
  while (current !== null) {
    if (current === node) return true;
    if (current.parentNode !== null) {
      current = current.parentNode;
      continue;
    }
    const root = current.getRootNode();
    current = "host" in root && root.host instanceof Node ? root.host : null;
  }
  return false;
}

export class FeedbackRuntime {
  readonly #clock: RuntimeClock;
  readonly #scheduler: RuntimeScheduler;
  readonly #records = new Map<IslandRecord, RecordFeedback>();

  constructor(clock: RuntimeClock, scheduler: RuntimeScheduler) {
    this.#clock = clock;
    this.#scheduler = scheduler;
  }

  connect(
    record: IslandRecord,
    directives: readonly OwnedDirective[],
    model: ModelState | null,
  ): void {
    if (this.#records.has(record)) return;
    const state: RecordFeedback = {
      bindings: new Set(),
      model,
      presentations: new Map(),
      recovery: "none",
      unsubscribeModel:
        model?.subscribe(() => {
          this.#update(record);
        }) ?? null,
      unsubscribeScheduler: record.scheduler.subscribeFeedback(() => {
        this.#update(record);
      }),
    };
    this.#records.set(record, state);
    this.scanInsertion(record, directives);
    record.onDispose(() => {
      this.retire(record);
    });
  }

  setRecovery(record: IslandRecord, recovery: RecoveryState): void {
    const state = this.#records.get(record);
    if (state === undefined || state.recovery === recovery) return;
    state.recovery = recovery;
    this.#update(record);
  }

  scanInsertion(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    for (const owned of directives) {
      if (!FEEDBACK_STATES.has(owned.directive.name)) continue;
      if (
        [...state.bindings].some(
          (managed) =>
            managed.binding.element() === owned.element &&
            managed.attributeName === owned.attributeName,
        )
      ) {
        continue;
      }
      const feedbackState = owned.directive.name as FeedbackState;
      const scope = scopeFor(record, state.model, owned.directive.value);
      let presentation = state.presentations.get(owned.element);
      if (presentation === undefined) {
        presentation = new FeedbackElementPresentation(owned.element, this.#scheduler);
        state.presentations.set(owned.element, presentation);
      }
      state.bindings.add({
        attributeName: owned.attributeName,
        binding: new FeedbackTargetBinding(
          owned.element,
          feedbackState,
          owned.directive.modifiers,
          `${scope.kind}:${scope.value}`,
          this.#clock,
          this.#scheduler,
          presentation,
        ),
        scope,
        state: feedbackState,
      });
    }
    this.#update(record);
  }

  retireSubtree(record: IslandRecord, node: Node): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    for (const managed of [...state.bindings]) {
      if (!within(node, managed.binding.element())) continue;
      managed.binding.dispose();
      state.bindings.delete(managed);
      if (managed.binding.presentationEmpty()) {
        state.presentations.delete(managed.binding.element());
      }
    }
  }

  retire(record: IslandRecord): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    state.unsubscribeModel?.();
    state.unsubscribeScheduler();
    for (const managed of state.bindings) managed.binding.dispose();
    state.bindings.clear();
    state.presentations.clear();
    this.#records.delete(record);
  }

  #update(record: IslandRecord): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    const records = record.scheduler.feedback();
    for (const managed of state.bindings) {
      const snapshot = projectFeedback(records, state.model, managed.scope, state.recovery);
      managed.binding.update(
        snapshot,
        transitionFor(snapshot, managed.scope, state.model),
        announcementFor(managed.state, managed.scope, state.model),
      );
    }
  }
}
