import type { RuntimeClock, RuntimeScheduler } from "../runtime/ports.js";
import type { BuiltLiveRequest } from "./request.js";
import { LiveTransportError, type LiveTransportResponse } from "./state.js";

export interface RetryPolicy {
  readonly maximumAttempts: number;
  readonly baseDelayMs: number;
  readonly maximumDelayMs: number;
  readonly jitterRatio: number;
  readonly retryableStatuses: readonly number[];
}

export interface LiveRetryOptions {
  readonly policy: RetryPolicy;
  readonly attempt: (
    request: BuiltLiveRequest,
    signal?: AbortSignal,
  ) => Promise<LiveTransportResponse>;
  readonly clock: RuntimeClock;
  readonly scheduler: RuntimeScheduler;
  readonly jitter: () => number;
  readonly isOnline: () => boolean;
  readonly signal?: AbortSignal;
}

export interface LiveRetryResult extends LiveTransportResponse {
  readonly attempts: number;
  readonly startedAt: number;
  readonly settledAt: number;
}

const MAX_ATTEMPTS = 16;
const MAX_DELAY_MS = 120_000;
const MAX_RETRYABLE_STATUSES = 16;

function validatePolicy(policy: RetryPolicy): void {
  if (
    !Number.isSafeInteger(policy.maximumAttempts) ||
    policy.maximumAttempts < 1 ||
    policy.maximumAttempts > MAX_ATTEMPTS ||
    !Number.isSafeInteger(policy.baseDelayMs) ||
    policy.baseDelayMs < 0 ||
    policy.baseDelayMs > MAX_DELAY_MS ||
    !Number.isSafeInteger(policy.maximumDelayMs) ||
    policy.maximumDelayMs < policy.baseDelayMs ||
    policy.maximumDelayMs > MAX_DELAY_MS ||
    !Number.isFinite(policy.jitterRatio) ||
    policy.jitterRatio < 0 ||
    policy.jitterRatio > 1 ||
    policy.retryableStatuses.length > MAX_RETRYABLE_STATUSES ||
    policy.retryableStatuses.some(
      (status) => !Number.isSafeInteger(status) || status < 400 || status > 599,
    ) ||
    new Set(policy.retryableStatuses).size !== policy.retryableStatuses.length
  ) {
    throw new LiveTransportError("network");
  }
}

function snapshotPolicy(policy: RetryPolicy): RetryPolicy {
  validatePolicy(policy);
  return Object.freeze({
    baseDelayMs: policy.baseDelayMs,
    jitterRatio: policy.jitterRatio,
    maximumAttempts: policy.maximumAttempts,
    maximumDelayMs: policy.maximumDelayMs,
    retryableStatuses: Object.freeze([...policy.retryableStatuses]),
  });
}

function now(clock: RuntimeClock): number {
  const value = clock.now();
  if (!Number.isFinite(value)) throw new LiveTransportError("network");
  return value;
}

function retryable(error: LiveTransportError, policy: RetryPolicy, online: () => boolean): boolean {
  if (error.kind === "http") {
    return error.status !== null && policy.retryableStatuses.includes(error.status);
  }
  if (error.kind === "network" || error.kind === "timeout") return true;
  if (error.kind === "offline") {
    try {
      return online();
    } catch {
      return false;
    }
  }
  return false;
}

function delayFor(attempts: number, policy: RetryPolicy, jitter: () => number): number {
  const exponent = Math.min(attempts - 1, 30);
  const bounded = Math.min(policy.maximumDelayMs, policy.baseDelayMs * 2 ** exponent);
  const sample = jitter();
  if (!Number.isFinite(sample) || sample < -1 || sample > 1) {
    throw new LiveTransportError("network");
  }
  return Math.max(
    0,
    Math.min(policy.maximumDelayMs, Math.round(bounded * (1 + sample * policy.jitterRatio))),
  );
}

function wait(
  milliseconds: number,
  scheduler: RuntimeScheduler,
  signal: AbortSignal | undefined,
): Promise<void> {
  if (signal?.aborted === true) return Promise.reject(new LiveTransportError("aborted"));
  return new Promise((resolve, reject) => {
    let settled = false;
    let handle: number | null = null;
    const cleanup = (): void => {
      signal?.removeEventListener("abort", abort);
    };
    const abort = (): void => {
      if (settled) return;
      settled = true;
      if (handle !== null) {
        try {
          scheduler.clearTimeout(handle);
        } catch {
          // Cancellation remains terminal even if the host timer port fails cleanup.
        }
      }
      cleanup();
      reject(new LiveTransportError("aborted"));
    };
    signal?.addEventListener("abort", abort, { once: true });
    try {
      handle = scheduler.timeout(() => {
        if (settled) return;
        settled = true;
        cleanup();
        if (signal?.aborted === true) reject(new LiveTransportError("aborted"));
        else resolve();
      }, milliseconds);
    } catch {
      settled = true;
      cleanup();
      reject(new LiveTransportError("network"));
    }
  });
}

function aborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

export async function retryLiveRequest(
  request: BuiltLiveRequest,
  options: LiveRetryOptions,
): Promise<LiveRetryResult> {
  const policy = snapshotPolicy(options.policy);
  const startedAt = now(options.clock);
  let attempts = 0;
  while (attempts < policy.maximumAttempts) {
    if (aborted(options.signal)) throw new LiveTransportError("aborted");
    attempts += 1;
    try {
      const response = await options.attempt(request, options.signal);
      if (aborted(options.signal)) throw new LiveTransportError("aborted");
      return Object.freeze({ ...response, attempts, settledAt: now(options.clock), startedAt });
    } catch (error: unknown) {
      const failure =
        error instanceof LiveTransportError ? error : new LiveTransportError("network");
      if (attempts >= policy.maximumAttempts || !retryable(failure, policy, options.isOnline)) {
        throw failure;
      }
      await wait(delayFor(attempts, policy, options.jitter), options.scheduler, options.signal);
    }
  }
  throw new LiveTransportError("network");
}
