import type { RuntimeClock, RuntimeScheduler } from "../runtime/ports.js";

export interface FeedbackTimingPolicy {
  readonly delayMs: number;
  readonly minimumVisibleMs: number;
  readonly resetMs: number | null;
}

const MAX_FEEDBACK_DURATION_MS = 120_000;

function validDuration(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0 && value <= MAX_FEEDBACK_DURATION_MS;
}

export class FeedbackTiming {
  readonly #clock: RuntimeClock;
  readonly #scheduler: RuntimeScheduler;
  readonly #policy: FeedbackTimingPolicy;
  readonly #onVisibility: (visible: boolean) => void;
  #visible = false;
  #visibleAt = 0;
  #transition: string | null = null;
  #suppressedTransition: string | null = null;
  #delayHandle: number | null = null;
  #hideHandle: number | null = null;
  #resetHandle: number | null = null;
  #disposed = false;

  constructor(
    clock: RuntimeClock,
    scheduler: RuntimeScheduler,
    policy: FeedbackTimingPolicy,
    onVisibility: (visible: boolean) => void,
  ) {
    if (
      !validDuration(policy.delayMs) ||
      !validDuration(policy.minimumVisibleMs) ||
      (policy.resetMs !== null && !validDuration(policy.resetMs))
    ) {
      throw new Error("feedback_timing_invalid");
    }
    this.#clock = clock;
    this.#scheduler = scheduler;
    this.#policy = Object.freeze({ ...policy });
    this.#onVisibility = onVisibility;
  }

  visible(): boolean {
    return this.#visible;
  }

  update(active: boolean, transition: string | null): void {
    if (this.#disposed) return;
    if (!active || transition === null) {
      this.#transition = null;
      this.#cancel("delay");
      this.#cancel("reset");
      this.#requestHide();
      return;
    }
    if (this.#suppressedTransition === transition) return;
    const changed = this.#transition !== transition;
    this.#transition = transition;
    this.#cancel("hide");
    if (changed) this.#cancel("reset");
    if (this.#visible) {
      if (changed) this.#scheduleReset(transition);
      return;
    }
    if (this.#delayHandle !== null) return;
    if (this.#policy.delayMs === 0) this.#show(transition);
    else {
      this.#delayHandle = this.#schedule(() => {
        this.#delayHandle = null;
        if (this.#transition === transition) this.#show(transition);
      }, this.#policy.delayMs);
    }
  }

  suspend(): void {
    if (this.#disposed) return;
    this.#cancel("delay");
    this.#cancel("hide");
    this.#cancel("reset");
    this.#transition = null;
    if (this.#visible) this.#setVisible(false);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.suspend();
    this.#disposed = true;
  }

  #show(transition: string): void {
    if (this.#visible || this.#disposed || this.#transition !== transition) return;
    this.#visibleAt = this.#now();
    this.#setVisible(true);
    this.#scheduleReset(transition);
  }

  #requestHide(): void {
    if (!this.#visible) return;
    const remaining = Math.max(
      0,
      this.#policy.minimumVisibleMs - Math.max(0, this.#now() - this.#visibleAt),
    );
    if (remaining === 0) this.#setVisible(false);
    else if (this.#hideHandle === null) {
      this.#hideHandle = this.#schedule(() => {
        this.#hideHandle = null;
        this.#setVisible(false);
      }, remaining);
    }
  }

  #scheduleReset(transition: string): void {
    if (this.#policy.resetMs === null) return;
    this.#cancel("reset");
    this.#resetHandle = this.#schedule(() => {
      this.#resetHandle = null;
      if (this.#transition !== transition) return;
      this.#suppressedTransition = transition;
      this.#transition = null;
      this.#requestHide();
    }, this.#policy.resetMs);
  }

  #setVisible(visible: boolean): void {
    if (this.#visible === visible) return;
    this.#visible = visible;
    this.#onVisibility(visible);
  }

  #now(): number {
    const value = this.#clock.now();
    if (!Number.isFinite(value)) throw new Error("feedback_clock_invalid");
    return value;
  }

  #schedule(callback: VoidFunction, delay: number): number {
    return this.#scheduler.timeout(callback, delay);
  }

  #cancel(kind: "delay" | "hide" | "reset"): void {
    const handle =
      kind === "delay" ? this.#delayHandle : kind === "hide" ? this.#hideHandle : this.#resetHandle;
    if (handle === null) return;
    try {
      this.#scheduler.clearTimeout(handle);
    } catch {
      // A failed cleanup port cannot reverse authoritative feedback state.
    }
    if (kind === "delay") this.#delayHandle = null;
    else if (kind === "hide") this.#hideHandle = null;
    else this.#resetHandle = null;
  }
}
