export type CoreResourceKind =
  | "authorization"
  | "buffer"
  | "controller"
  | "extension"
  | "listener"
  | "membership"
  | "observer"
  | "queue"
  | "scheduler"
  | "signal"
  | "timer"
  | "transition"
  | "transport";

export type FeatureResourceKind = "upload" | "stream" | "poll";
export type ResourceKind = CoreResourceKind | FeatureResourceKind;
export type ResourceCounts = Readonly<Record<CoreResourceKind, number>>;

export interface Disposable {
  dispose(): void;
}

export interface LifecycleResource {
  readonly dispose: () => void;
  readonly resume?: () => void;
  readonly suspend?: () => void;
}

export interface ResourceLedger {
  add(kind: CoreResourceKind, dispose: () => void): Disposable;
  suspend(): void;
  resume(): void;
  dispose(): void;
  counts(): ResourceCounts;
}

export interface ResourceLedgerOptions {
  readonly maxResources?: number;
}

type LedgerState = "created" | "active" | "suspended" | "disposed";

interface ResourceEntry {
  readonly kind: CoreResourceKind;
  readonly resource: LifecycleResource;
  active: boolean;
}

const RESOURCE_KINDS = [
  "authorization",
  "buffer",
  "controller",
  "extension",
  "listener",
  "membership",
  "observer",
  "queue",
  "scheduler",
  "signal",
  "timer",
  "transition",
  "transport",
] as const satisfies readonly CoreResourceKind[];

const DEFAULT_MAX_RESOURCES = 2_048;
const RESOURCE_LEDGERS = new WeakMap<object, ResourceLedger>();

function invoke(callback: (() => void) | undefined): void {
  try {
    callback?.();
  } catch {
    // Lifecycle cleanup is best effort, bounded, and never invokes one resource twice per edge.
  }
}

export class ResourceLedgerImpl implements ResourceLedger {
  readonly #entries = new Set<ResourceEntry>();
  readonly #maxResources: number;
  #state: LedgerState = "created";

  constructor(options: ResourceLedgerOptions = {}) {
    const maxResources = options.maxResources ?? DEFAULT_MAX_RESOURCES;
    if (!Number.isSafeInteger(maxResources) || maxResources < 1 || maxResources > 16_384) {
      throw new RangeError("resource_ledger_limit");
    }
    this.#maxResources = maxResources;
  }

  add(kind: CoreResourceKind, dispose: () => void): Disposable {
    return this.track(kind, { dispose });
  }

  track(kind: CoreResourceKind, resource: LifecycleResource): Disposable {
    if (this.#state === "disposed") throw new Error("resource_ledger_disposed");
    if (this.#entries.size >= this.#maxResources) throw new Error("resource_ledger_capacity");
    const entry: ResourceEntry = { active: true, kind, resource };
    this.#entries.add(entry);
    if (this.#state === "active") invoke(resource.resume);
    if (this.#state === "suspended") invoke(resource.suspend);
    return Object.freeze({
      dispose: () => {
        if (!entry.active) return;
        entry.active = false;
        this.#entries.delete(entry);
        invoke(entry.resource.dispose);
      },
    });
  }

  suspend(): void {
    if (this.#state !== "active") return;
    for (const entry of [...this.#entries].reverse()) {
      if (entry.active) invoke(entry.resource.suspend);
    }
    this.#state = "suspended";
  }

  resume(): void {
    if (this.#state === "disposed" || this.#state === "active") return;
    for (const entry of this.#entries) if (entry.active) invoke(entry.resource.resume);
    this.#state = "active";
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    for (const entry of [...this.#entries].reverse()) {
      if (!entry.active) continue;
      entry.active = false;
      this.#entries.delete(entry);
      invoke(entry.resource.dispose);
    }
  }

  counts(): ResourceCounts {
    const counts = Object.fromEntries(RESOURCE_KINDS.map((kind) => [kind, 0])) as Record<
      CoreResourceKind,
      number
    >;
    for (const entry of this.#entries) if (entry.active) counts[entry.kind] += 1;
    return Object.freeze(counts);
  }
}

export function bindResourceLedger(owner: object, ledger: ResourceLedger): void {
  if (RESOURCE_LEDGERS.has(owner)) throw new Error("resource_ledger_owner_duplicate");
  RESOURCE_LEDGERS.set(owner, ledger);
}

export function boundResourceLedger(owner: object): ResourceLedger | null {
  return RESOURCE_LEDGERS.get(owner) ?? null;
}
