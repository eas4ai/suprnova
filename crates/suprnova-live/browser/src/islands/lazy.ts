import type { ParsedDirective } from "../directives/types.js";
import type { RuntimeObserverFactory, RuntimeRandomness } from "../runtime/ports.js";
import { createServerIntent, type IntentSource } from "../scheduler/intent.js";
import type { IslandRecord } from "./record.js";

export type LazyMarkerState = "pending" | "queued" | "resolved" | "retired";

export class LazyIntentMarker {
  #state: LazyMarkerState = "pending";

  state(): LazyMarkerState {
    return this.#state;
  }

  queue(): boolean {
    if (this.#state !== "pending") return false;
    this.#state = "queued";
    return true;
  }

  resolve(): void {
    if (this.#state === "queued") this.#state = "resolved";
  }

  retire(): void {
    this.#state = "retired";
  }
}

export interface LazyDirectiveCandidate {
  readonly directive: ParsedDirective;
  readonly element: Element;
}

interface LazyEntry {
  readonly record: IslandRecord;
  readonly candidate: LazyDirectiveCandidate;
  readonly marker: LazyIntentMarker;
}

export class LazyCoordinator {
  readonly #randomness: RuntimeRandomness;
  readonly #observer: IntersectionObserver | null;
  readonly #entries = new Map<string, LazyEntry>();
  readonly #byTarget = new WeakMap<Element, LazyEntry>();
  #suspended = false;

  constructor(observers: RuntimeObserverFactory, randomness: RuntimeRandomness) {
    this.#randomness = randomness;
    this.#observer = observers.intersection((entries) => {
      for (const observed of entries) {
        if (observed.isIntersecting) this.#queue(this.#byTarget.get(observed.target));
      }
    });
  }

  connect(record: IslandRecord, candidates: readonly LazyDirectiveCandidate[]): void {
    if (record.metadata.lazyComplete || this.#entries.has(record.metadata.documentKey)) return;
    const candidate = candidates.find((entry) => entry.directive.name === "lazy");
    if (candidate === undefined) return;
    const entry = { record, candidate, marker: new LazyIntentMarker() };
    this.#entries.set(record.metadata.documentKey, entry);
    this.#byTarget.set(candidate.element, entry);
    record.onDispose(() => {
      this.#retire(entry);
    });
    if (candidate.directive.modifiers.includes("eager")) this.#queue(entry);
    else if (!this.#suspended) this.#observer?.observe(candidate.element);
  }

  suspend(): void {
    if (this.#suspended) return;
    this.#suspended = true;
    this.#observer?.disconnect();
  }

  resume(): void {
    if (!this.#suspended) return;
    this.#suspended = false;
    for (const entry of this.#entries.values()) {
      if (entry.marker.state() === "pending") this.#observer?.observe(entry.candidate.element);
    }
  }

  dispose(): void {
    this.#observer?.disconnect();
    for (const entry of this.#entries.values()) entry.marker.retire();
    this.#entries.clear();
  }

  #queue(entry: LazyEntry | undefined): void {
    if (entry === undefined || !entry.marker.queue() || !entry.record.active()) return;
    const source: IntentSource = Object.freeze({
      island: entry.record,
      element: entry.candidate.element,
      directive: entry.candidate.directive,
      eventType: "lazy",
      trusted: true,
    });
    try {
      const intent = createServerIntent(
        source,
        [{ kind: "lazy_complete" }],
        this.#randomness,
        entry.record.metadata.snapshotForm === "seed",
      );
      intent.onFinish(() => {
        entry.marker.resolve();
      });
      if (!entry.record.enqueue(intent)) intent.finish("rejected");
    } catch {
      entry.marker.retire();
    }
  }

  #retire(entry: LazyEntry): void {
    this.#observer?.unobserve(entry.candidate.element);
    this.#entries.delete(entry.record.metadata.documentKey);
    this.#byTarget.delete(entry.candidate.element);
    entry.marker.retire();
  }
}
