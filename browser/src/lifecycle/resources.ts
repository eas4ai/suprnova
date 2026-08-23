export type CoreResourceKind =
  | "controller"
  | "extension"
  | "listener"
  | "observer"
  | "scheduler"
  | "signal"
  | "timer"
  | "transition"
  | "transport";

export type FeatureResourceKind = "upload" | "stream" | "poll";
export type ResourceKind = CoreResourceKind | FeatureResourceKind;
export type ResourceCounts<Kind extends ResourceKind = CoreResourceKind> = Readonly<
  Record<Kind, number>
>;

export interface Disposable {
  dispose(): void;
}

export interface LifecycleResource {
  readonly dispose: () => void;
  readonly resume?: () => void;
  readonly suspend?: () => void;
}

export interface ResourceLedger<Kind extends ResourceKind = CoreResourceKind> {
  add(kind: Kind, dispose: () => void): Disposable;
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
  "controller",
  "extension",
  "listener",
  "observer",
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
  readonly #entries: ResourceEntry[] = [];
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
    if (this.#entries.length >= this.#maxResources) throw new Error("resource_ledger_capacity");
    const entry: ResourceEntry = { active: true, kind, resource };
    this.#entries.push(entry);
    if (this.#state === "active") invoke(resource.resume);
    if (this.#state === "suspended") invoke(resource.suspend);
    return Object.freeze({
      dispose: () => {
        if (!entry.active) return;
        entry.active = false;
        invoke(entry.resource.dispose);
      },
    });
  }

  suspend(): void {
    if (this.#state !== "active") return;
    for (let index = this.#entries.length - 1; index >= 0; index -= 1) {
      const entry = this.#entries[index];
      if (entry?.active === true) invoke(entry.resource.suspend);
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
    for (let index = this.#entries.length - 1; index >= 0; index -= 1) {
      const entry = this.#entries[index];
      if (entry?.active !== true) continue;
      entry.active = false;
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
