import type { FreshRenderCompletionObserver, FreshRenderDisposition } from "../features/host.js";
import { freshnessCombination, type FreshnessStreamMode } from "../generated/directive-contract.js";
import type { AsyncRandomness, AsyncTimerPort, PollFallbackPolicy } from "./types.js";

const MIN_POLL_INTERVAL_MS = 1_000;
const MAX_POLL_INTERVAL_MS = 300_000;
const DEFAULT_POLL_INTERVAL_MS = 30_000;
const DEFAULT_POLL_JITTER_RATIO = 0.2;

export interface PollPolicy {
  readonly intervalMs: number;
  readonly jitterRatio: number;
  readonly initial: "wait" | "immediate";
  readonly visibility: "visible" | "always";
  readonly mode: "poll_only" | "push_only" | "hybrid";
}

export interface PollEnvironment {
  isOnline(): boolean;
  isVisible(): boolean;
  subscribe(listener: VoidFunction): VoidFunction;
}

export type PollStatus = "current" | "degraded" | "polling" | "offline" | "suspended" | "closed";

export interface PollTimerOptions {
  readonly enqueueFreshRender: (
    reason: "poll",
    completion: FreshRenderCompletionObserver,
    completionKey?: string,
  ) => FreshRenderDisposition;
  readonly environment: PollEnvironment;
  readonly observe?: ((state: PollStatus) => void) | undefined;
  readonly policy: PollPolicy;
  readonly randomness: AsyncRandomness;
  readonly timers: AsyncTimerPort;
}

function validPolicy(policy: unknown): policy is PollPolicy | PollFallbackPolicy {
  try {
    const value = policy as Readonly<Record<string, unknown>>;
    const interval = value["intervalMs"];
    const jitter = value["jitterRatio"];
    return (
      Number.isSafeInteger(interval) &&
      typeof interval === "number" &&
      interval >= MIN_POLL_INTERVAL_MS &&
      interval <= MAX_POLL_INTERVAL_MS &&
      Number.isFinite(jitter) &&
      typeof jitter === "number" &&
      jitter >= 0 &&
      jitter <= 1 &&
      (value["initial"] === "wait" || value["initial"] === "immediate") &&
      (value["visibility"] === "visible" || value["visibility"] === "always")
    );
  } catch {
    return false;
  }
}

function pollInterval(modifiers: readonly string[]): number {
  const interval = modifiers.find((modifier) => /^\d+s$/u.test(modifier));
  return interval === undefined ? DEFAULT_POLL_INTERVAL_MS : Number.parseInt(interval, 10) * 1_000;
}

function policyFromPoll(
  modifiers: readonly string[],
  mode: PollPolicy["mode"],
  fallback: PollFallbackPolicy | null,
): PollPolicy {
  const policy = Object.freeze({
    initial: modifiers.includes("immediate")
      ? ("immediate" as const)
      : (fallback?.initial ?? "wait"),
    intervalMs: pollInterval(modifiers),
    jitterRatio: fallback?.jitterRatio ?? DEFAULT_POLL_JITTER_RATIO,
    mode,
    visibility: modifiers.includes("always")
      ? ("always" as const)
      : modifiers.includes("visible")
        ? ("visible" as const)
        : (fallback?.visibility ?? "visible"),
  });
  if (!validPolicy(policy)) throw new Error("poll_policy_invalid");
  return policy;
}

function streamMode(modifiers: readonly string[] | null): FreshnessStreamMode {
  if (modifiers === null) return "absent";
  if (modifiers.includes("push-only")) return "push-only";
  if (modifiers.includes("hybrid")) return "hybrid";
  return "default";
}

export function resolvePollPolicy(
  pollModifiers: readonly string[] | null,
  streamModifiers: readonly string[] | null,
  fallback: PollFallbackPolicy | null,
): PollPolicy | null {
  const combination = freshnessCombination(pollModifiers !== null, streamMode(streamModifiers));
  switch (combination) {
    case "none":
      return null;
    case "poll_only":
      return policyFromPoll(pollModifiers ?? [], "poll_only", null);
    case "hybrid_descriptor":
      if (fallback === null || !validPolicy(fallback)) throw new Error("poll_policy_invalid");
      return Object.freeze({ ...fallback, mode: "hybrid" });
    case "hybrid_poll_override":
      if (fallback === null || !validPolicy(fallback)) throw new Error("poll_policy_invalid");
      return policyFromPoll(pollModifiers ?? [], "hybrid", fallback);
    case "push_only":
      if (fallback !== null && !validPolicy(fallback)) throw new Error("poll_policy_invalid");
      return Object.freeze({
        ...(fallback ?? {
          initial: "wait",
          intervalMs: DEFAULT_POLL_INTERVAL_MS,
          jitterRatio: DEFAULT_POLL_JITTER_RATIO,
          visibility: "visible",
        }),
        mode: "push_only",
      });
    case "directive_conflict":
      throw new Error("directive_conflict");
    case undefined:
      throw new Error("poll_policy_invalid");
  }
}

export class PollTimer {
  readonly #enqueueFreshRender: PollTimerOptions["enqueueFreshRender"];
  readonly #environment: PollEnvironment;
  readonly #randomness: AsyncRandomness;
  readonly #timers: AsyncTimerPort;
  #environmentDisposer: VoidFunction | null = null;
  #failures = 0;
  #generation = 0;
  #handle: number | null = null;
  readonly #observe: ((state: PollStatus) => void) | undefined;
  #observedState: PollStatus | null = null;
  #policy: PollPolicy;
  #requestPending = false;
  #started = false;
  #state: PollStatus;
  #suspended = false;
  #continuityCurrent = false;

  constructor(options: PollTimerOptions) {
    if (!validPolicy(options.policy)) throw new Error("poll_policy_invalid");
    this.#enqueueFreshRender = options.enqueueFreshRender;
    this.#environment = options.environment;
    this.#observe = options.observe;
    this.#policy = options.policy;
    this.#randomness = options.randomness;
    this.#state = this.#freshnessState();
    this.#timers = options.timers;
  }

  start(): void {
    if (this.#started || this.#state === "closed") return;
    this.#started = true;
    this.#environmentDisposer = this.#environment.subscribe(() => {
      this.#environmentChanged();
    });
    if (this.#suspended) return;
    this.#state = this.#freshnessState();
    this.#emitState();
    if (this.#policy.mode === "push_only") return;
    if (this.#policy.initial === "immediate" && this.#eligible()) this.#tick();
    else if (this.#eligible()) this.#arm(false);
  }

  status(): PollStatus {
    return this.#state;
  }

  continuity(state: "current" | "degraded"): void {
    if (this.#state === "closed" || this.#policy.mode === "poll_only") return;
    this.#continuityCurrent = state === "current";
    if (this.#continuityCurrent) {
      this.#clear();
      this.#fenceRequest();
      this.#setState(this.#freshnessState());
      return;
    }
    this.#setState(this.#freshnessState());
    if (this.#policy.mode === "hybrid" && this.#started && !this.#suspended && this.#eligible()) {
      this.#arm(false);
    }
  }

  updatePolicy(policy: PollPolicy, applyInitial = false): void {
    if (!validPolicy(policy)) throw new Error("poll_policy_invalid");
    if (this.#state === "closed") return;
    const current = this.#policy;
    if (
      current.initial === policy.initial &&
      current.intervalMs === policy.intervalMs &&
      current.jitterRatio === policy.jitterRatio &&
      current.mode === policy.mode &&
      current.visibility === policy.visibility
    ) {
      return;
    }
    this.#policy = policy;
    this.#clear();
    this.#fenceRequest();
    this.#setState(this.#freshnessState());
    if (this.#started && !this.#suspended && this.#shouldPoll() && this.#eligible()) {
      if (applyInitial && this.#policy.initial === "immediate") this.#tick();
      else this.#arm(false);
    }
  }

  suspend(): void {
    if (this.#state === "closed" || this.#suspended) return;
    this.#suspended = true;
    this.#clear();
    this.#fenceRequest();
    this.#setState("suspended");
  }

  resume(): void {
    if (this.#state === "closed" || !this.#suspended) return;
    this.#suspended = false;
    this.#setState(this.#freshnessState());
    if (this.#shouldPoll() && this.#eligible()) this.#arm(false);
  }

  dispose(): void {
    if (this.#state === "closed") return;
    this.#clear();
    this.#fenceRequest();
    this.#environmentDisposer?.();
    this.#environmentDisposer = null;
    this.#setState("closed");
  }

  #environmentChanged(): void {
    if (!this.#started || this.#suspended || this.#state === "closed") {
      return;
    }
    this.#clear();
    this.#setState(this.#freshnessState());
    if (this.#state === "offline" || this.#state === "current") return;
    if (this.#shouldPoll() && this.#eligible()) this.#arm(false);
  }

  #freshnessState(): PollStatus {
    if (this.#suspended) return "suspended";
    if (!this.#environment.isOnline()) return "offline";
    return this.#continuityCurrent && this.#policy.mode !== "poll_only" ? "current" : "degraded";
  }

  #eligible(): boolean {
    return (
      this.#environment.isOnline() &&
      (this.#policy.visibility === "always" || this.#environment.isVisible())
    );
  }

  #shouldPoll(): boolean {
    return (
      this.#policy.mode === "poll_only" ||
      (this.#policy.mode === "hybrid" && !this.#continuityCurrent)
    );
  }

  #random(): number {
    const value = this.#randomness.number();
    if (!Number.isFinite(value) || value < 0 || value >= 1) {
      throw new Error("async_randomness_invalid");
    }
    return value;
  }

  #delay(backoff: boolean): number {
    const random = this.#random();
    if (!backoff) {
      return (
        this.#policy.intervalMs +
        Math.floor(this.#policy.intervalMs * this.#policy.jitterRatio * random)
      );
    }
    const exponent = Math.min(this.#failures, 16);
    const maximum = Math.min(MAX_POLL_INTERVAL_MS, this.#policy.intervalMs * 2 ** exponent);
    return Math.max(1, Math.floor(maximum * random));
  }

  #arm(backoff: boolean): void {
    if (
      this.#handle !== null ||
      !this.#shouldPoll() ||
      this.#state === "closed" ||
      this.#suspended ||
      this.#requestPending ||
      !this.#eligible()
    ) {
      return;
    }
    const delay = this.#delay(backoff);
    this.#handle = this.#timers.timeout(() => {
      this.#handle = null;
      this.#tick();
    }, delay);
  }

  #clear(): void {
    if (this.#handle === null) return;
    this.#timers.clearTimeout(this.#handle);
    this.#handle = null;
  }

  #tick(): void {
    if (
      !this.#shouldPoll() ||
      this.#state === "closed" ||
      this.#suspended ||
      this.#requestPending
    ) {
      return;
    }
    if (!this.#eligible()) {
      this.#setState(this.#environment.isOnline() ? "degraded" : "offline");
      return;
    }
    const generation = ++this.#generation;
    this.#requestPending = true;
    this.#setState("polling");
    try {
      const disposition = this.#enqueueFreshRender(
        "poll",
        (completion) => {
          this.#complete(generation, completion);
        },
        "poll",
      );
      if (disposition === "retired") {
        this.#complete(generation, "retired");
        return;
      }
    } catch {
      if (!this.#requestCurrent(generation)) return;
      this.#requestPending = false;
      this.#failures += 1;
      this.#setState(this.#environment.isOnline() ? "degraded" : "offline");
      this.#arm(true);
    }
  }

  #complete(generation: number, completion: Parameters<FreshRenderCompletionObserver>[0]): void {
    if (!this.#requestCurrent(generation) || this.#state === "closed") {
      return;
    }
    this.#requestPending = false;
    if (completion === "retired") {
      this.dispose();
      return;
    }
    if (completion === "succeeded") {
      this.#failures = 0;
      this.#setState("current");
      this.#arm(false);
      return;
    }
    this.#failures += 1;
    this.#setState(this.#environment.isOnline() ? "degraded" : "offline");
    this.#arm(true);
  }

  #fenceRequest(): void {
    this.#generation += 1;
    this.#requestPending = false;
  }

  #requestCurrent(generation: number): boolean {
    return this.#generation === generation && this.#requestPending;
  }

  #setState(state: PollStatus): void {
    if (this.#state === state) return;
    this.#state = state;
    this.#emitState();
  }

  #emitState(): void {
    if (this.#observedState === this.#state) return;
    this.#observedState = this.#state;
    try {
      this.#observe?.(this.#state);
    } catch {
      // A semantic observer cannot affect freshness authority or scheduling.
    }
  }
}
