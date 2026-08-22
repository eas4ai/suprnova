import type { RuntimeClock, RuntimeScheduler } from "../runtime/ports.js";

const MAX_TIMING_KEYS = 1_024;
const MAX_TIMING_KEY_UNITS = 512;
const SCHEDULING_MODIFIERS = new Set(["latest", "parallel", "serial"]);
const TIMING_MODIFIERS = new Map<string, ModelTimingPolicy>([
  ["immediate", Object.freeze({ kind: "immediate" })],
  ["change", Object.freeze({ kind: "change" })],
  ["blur", Object.freeze({ kind: "blur" })],
  ["action", Object.freeze({ kind: "action" })],
  ["submit", Object.freeze({ kind: "submit" })],
  ["debounce.100ms", Object.freeze({ kind: "debounce", milliseconds: 100 })],
  ["debounce.250ms", Object.freeze({ kind: "debounce", milliseconds: 250 })],
  ["debounce.500ms", Object.freeze({ kind: "debounce", milliseconds: 500 })],
  ["throttle.100ms", Object.freeze({ kind: "throttle", milliseconds: 100 })],
  ["throttle.250ms", Object.freeze({ kind: "throttle", milliseconds: 250 })],
]);

export type ModelTimingPolicy =
  | Readonly<{ kind: "immediate" | "change" | "blur" | "action" | "submit" }>
  | Readonly<{ kind: "debounce" | "throttle"; milliseconds: number }>;

export type ModelTimingEvent = "input" | "change" | "blur" | "reset";
export type ModelTimingDisposition = "invoked" | "deferred" | "ignored";

interface TimingEntry {
  callback: VoidFunction | null;
  handle: number | null;
  generation: number;
}

export function parseModelTiming(modifiers: readonly string[]): ModelTimingPolicy {
  let timing: ModelTimingPolicy | null = null;
  for (const modifier of modifiers) {
    if (SCHEDULING_MODIFIERS.has(modifier)) continue;
    const candidate = TIMING_MODIFIERS.get(modifier);
    if (candidate === undefined) throw new Error("model_timing_invalid");
    if (timing !== null) throw new Error("model_timing_conflict");
    timing = candidate;
  }
  return timing ?? Object.freeze({ kind: "immediate" });
}

export class ModelTimingCoordinator {
  readonly #clock: RuntimeClock;
  readonly #scheduler: RuntimeScheduler;
  readonly #maximum: number;
  readonly #entries = new Map<string, TimingEntry>();
  #generation = 0;
  #disposed = false;

  constructor(clock: RuntimeClock, scheduler: RuntimeScheduler, maximum = MAX_TIMING_KEYS) {
    if (!Number.isSafeInteger(maximum) || maximum < 1 || maximum > MAX_TIMING_KEYS) {
      throw new Error("model_timing_limit_invalid");
    }
    this.#clock = clock;
    this.#scheduler = scheduler;
    this.#maximum = maximum;
  }

  update(
    key: string,
    policy: ModelTimingPolicy,
    event: ModelTimingEvent,
    callback: VoidFunction,
  ): ModelTimingDisposition {
    this.#assertUsable(key, callback);
    switch (policy.kind) {
      case "immediate":
        this.cancel(key);
        safeInvoke(callback);
        return "invoked";
      case "change":
        return event === "change"
          ? this.#replaceAndFlush(key, callback)
          : this.#defer(key, callback);
      case "blur":
        return event === "blur" ? this.#replaceAndFlush(key, callback) : this.#defer(key, callback);
      case "action":
      case "submit":
        return this.#defer(key, callback);
      case "debounce":
        if (event !== "input" && event !== "change" && event !== "reset") return "ignored";
        return this.#debounce(key, policy.milliseconds, callback);
      case "throttle":
        if (event !== "input" && event !== "change" && event !== "reset") return "ignored";
        return this.#throttle(key, policy.milliseconds, callback);
    }
  }

  flush(key: string): boolean {
    const entry = this.#entries.get(key);
    if (entry?.callback === undefined || entry.callback === null) return false;
    this.#entries.delete(key);
    if (entry.handle !== null) safeClearTimeout(this.#scheduler, entry.handle);
    const callback = entry.callback;
    entry.callback = null;
    safeInvoke(callback);
    return true;
  }

  cancel(key: string): void {
    const entry = this.#entries.get(key);
    if (entry === undefined) return;
    this.#entries.delete(key);
    if (entry.handle !== null) safeClearTimeout(this.#scheduler, entry.handle);
    entry.callback = null;
  }

  pending(): number {
    let count = 0;
    for (const entry of this.#entries.values()) if (entry.callback !== null) count += 1;
    return count;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const key of [...this.#entries.keys()]) this.cancel(key);
  }

  #assertUsable(key: string, callback: VoidFunction): void {
    if (this.#disposed) throw new Error("model_timing_disposed");
    if (key.length === 0 || key.length > MAX_TIMING_KEY_UNITS || typeof callback !== "function") {
      throw new Error("model_timing_key_invalid");
    }
  }

  #entry(key: string): TimingEntry {
    let entry = this.#entries.get(key);
    if (entry !== undefined) return entry;
    if (this.#entries.size >= this.#maximum) throw new Error("model_timing_resource_limit");
    entry = { callback: null, generation: 0, handle: null };
    this.#entries.set(key, entry);
    return entry;
  }

  #defer(key: string, callback: VoidFunction): ModelTimingDisposition {
    const entry = this.#entry(key);
    entry.callback = callback;
    return "deferred";
  }

  #replaceAndFlush(key: string, callback: VoidFunction): ModelTimingDisposition {
    this.#defer(key, callback);
    this.flush(key);
    return "invoked";
  }

  #debounce(key: string, milliseconds: number, callback: VoidFunction): ModelTimingDisposition {
    const entry = this.#entry(key);
    if (entry.handle !== null) safeClearTimeout(this.#scheduler, entry.handle);
    entry.callback = callback;
    entry.generation = this.#nextGeneration();
    const generation = entry.generation;
    try {
      entry.handle = this.#scheduler.timeout(() => {
        const current = this.#entries.get(key);
        if (this.#disposed || current !== entry || current.generation !== generation) return;
        this.#entries.delete(key);
        const pending = current.callback;
        current.callback = null;
        current.handle = null;
        if (pending !== null) safeInvoke(pending);
      }, milliseconds);
    } catch (error: unknown) {
      this.#entries.delete(key);
      entry.callback = null;
      entry.handle = null;
      throw error;
    }
    return "deferred";
  }

  #throttle(key: string, milliseconds: number, callback: VoidFunction): ModelTimingDisposition {
    const existing = this.#entries.get(key);
    if (existing !== undefined) {
      existing.callback = callback;
      return "deferred";
    }
    const started = this.#clock.now();
    if (!Number.isFinite(started)) throw new Error("model_clock_invalid");
    const entry = this.#entry(key);
    entry.callback = null;
    entry.generation = this.#nextGeneration();
    const generation = entry.generation;
    try {
      entry.handle = this.#scheduler.timeout(() => {
        const current = this.#entries.get(key);
        if (this.#disposed || current !== entry || current.generation !== generation) return;
        this.#entries.delete(key);
        const pending = current.callback;
        current.callback = null;
        current.handle = null;
        if (pending !== null) safeInvoke(pending);
      }, milliseconds);
    } catch (error: unknown) {
      this.#entries.delete(key);
      entry.callback = null;
      entry.handle = null;
      throw error;
    }
    safeInvoke(callback);
    return "invoked";
  }

  #nextGeneration(): number {
    if (this.#generation >= Number.MAX_SAFE_INTEGER) this.#generation = 0;
    this.#generation += 1;
    return this.#generation;
  }
}

function safeInvoke(callback: VoidFunction): void {
  try {
    callback();
  } catch {
    // User-triggered timing work cannot escape into the browser event loop.
  }
}

function safeClearTimeout(scheduler: RuntimeScheduler, handle: number): void {
  try {
    scheduler.clearTimeout(handle);
  } catch {
    // A hostile host timer cannot retain or resurrect already-canceled model work.
  }
}
