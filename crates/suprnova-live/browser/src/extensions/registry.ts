import type { RuntimeScheduler } from "../runtime/ports.js";

const EXTENSION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const MAX_REGISTRATIONS = 128;
const MIN_DEADLINE_MS = 1;
const MAX_DEADLINE_MS = 5_000;

export class ExtensionError extends Error {
  constructor(readonly code: string) {
    super(code);
    this.name = "ExtensionError";
  }
}

export function validateExtensionName(name: unknown): asserts name is string {
  if (typeof name !== "string" || !EXTENSION_NAME.test(name)) {
    throw new ExtensionError("extension_name_invalid");
  }
}

export function requireExactRegistration(value: object, keys: readonly string[]): void {
  const present = Object.keys(value);
  if (present.length !== keys.length || present.some((key) => !keys.includes(key))) {
    throw new ExtensionError("extension_registration_shape");
  }
}

export interface RegistrationLease {
  active: boolean;
  readonly pending: Set<(status: "canceled") => void>;
}

export function createLease(): RegistrationLease {
  return { active: true, pending: new Set() };
}

export function disposeLease(lease: RegistrationLease): void {
  if (!lease.active) return;
  lease.active = false;
  for (const cancel of [...lease.pending]) cancel("canceled");
  lease.pending.clear();
}

export function assertRegistrationCapacity(size: number): void {
  if (size >= MAX_REGISTRATIONS) throw new ExtensionError("extension_registration_limit");
}

export type BoundedRunResult<Value> =
  | Readonly<{ status: "completed"; value: Value }>
  | Readonly<{ status: "failed" | "timeout" | "canceled" }>;

export class BoundedExtensionRunner {
  readonly #scheduler: RuntimeScheduler;
  readonly #deadlineMs: number;

  constructor(scheduler: RuntimeScheduler, deadlineMs = 1_000) {
    if (
      !Number.isSafeInteger(deadlineMs) ||
      deadlineMs < MIN_DEADLINE_MS ||
      deadlineMs > MAX_DEADLINE_MS
    ) {
      throw new ExtensionError("extension_deadline_invalid");
    }
    this.#scheduler = scheduler;
    this.#deadlineMs = deadlineMs;
  }

  run<Value>(
    lease: RegistrationLease,
    work: (active: () => boolean) => Value | Promise<Value>,
  ): Promise<BoundedRunResult<Value>> {
    if (!lease.active) return Promise.resolve(Object.freeze({ status: "canceled" }));
    return new Promise((resolve) => {
      let pending = true;
      let timeoutHandle = 0;
      const active = () => pending && lease.active;
      const finish = (result: BoundedRunResult<Value>) => {
        if (!pending) return;
        pending = false;
        lease.pending.delete(cancel);
        this.#scheduler.clearTimeout(timeoutHandle);
        resolve(Object.freeze(result));
      };
      const cancel = () => {
        finish({ status: "canceled" });
      };
      lease.pending.add(cancel);
      timeoutHandle = this.#scheduler.timeout(() => {
        finish({ status: "timeout" });
      }, this.#deadlineMs);
      Promise.resolve()
        .then(() => {
          if (!active()) throw new ExtensionError("extension_canceled");
          return work(active);
        })
        .then(
          (value) => {
            finish(active() ? { status: "completed", value } : { status: "canceled" });
          },
          () => {
            finish({ status: "failed" });
          },
        );
    });
  }
}

export interface IslandExtensionIdentity {
  readonly component: string;
  readonly slot: string;
  readonly documentKey: string;
}

export function normalizeIslandIdentity(value: IslandExtensionIdentity): IslandExtensionIdentity {
  for (const item of [value.component, value.slot, value.documentKey]) {
    if (typeof item !== "string" || item.length === 0 || item.length > 128) {
      throw new ExtensionError("extension_context_invalid");
    }
  }
  return Object.freeze({
    component: value.component,
    slot: value.slot,
    documentKey: value.documentKey,
  });
}
