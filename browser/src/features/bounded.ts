export const HARD_MAX_RESOURCE_ITEMS = 65_536;
export const HARD_MAX_RESOURCE_BYTES = 1024 * 1024 * 1024;
export const HARD_MAX_ACTIVE_PERMITS = 65_536;

export interface BoundedOwnerLimits {
  readonly maxItems: number;
  readonly maxBytes: number;
  readonly maxActive: number;
}

export type BoundedOwnerState = "active" | "suspended" | "retired";
export type QueueAdmission = "accepted" | "items_exceeded" | "bytes_exceeded" | "retired";
export type PermitRequestState = "waiting" | "admitted" | "canceled" | "items_exceeded" | "retired";

export interface BoundedDisposable {
  dispose(): void;
}

export type BoundedLease = BoundedDisposable;

export interface PermitRequest extends BoundedDisposable {
  state(): PermitRequestState;
}

export interface BoundedLifecycleResource {
  readonly dispose: () => void;
  readonly resume?: () => void;
  readonly suspend?: () => void;
}

export interface BoundedOwnerSnapshot {
  readonly state: BoundedOwnerState;
  readonly canceled: boolean;
  readonly queuedItems: number;
  readonly queuedBytes: number;
  readonly active: number;
  readonly waitingPermits: number;
  readonly ownedResources: number;
  readonly pendingResources: number;
}

export interface BoundedOwnerRetirement {
  readonly drainedItems: number;
  readonly drainedBytes: number;
  readonly releasedPermits: number;
}

interface QueuedItem<T> {
  readonly value: T;
  readonly bytes: number;
}

interface LeaseRecord {
  active: boolean;
}

interface PermitWaiter {
  readonly admit: (lease: BoundedLease) => void;
  state: PermitRequestState;
  lease: BoundedLease | null;
}

interface ResourceRecord {
  readonly dispose: () => void;
  readonly resume: (() => void) | undefined;
  readonly suspend: (() => void) | undefined;
  active: boolean;
  activated: boolean;
}

const CALLBACK_READ_FAILED = Symbol("bounded_owner_callback_read_failed");

function validLimit(value: number, maximum: number): boolean {
  return Number.isSafeInteger(value) && value >= 1 && value <= maximum;
}

function validItemBytes(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function isNullish(value: unknown): value is null | undefined {
  return value === null || value === undefined;
}

function isCallback(value: unknown): value is () => void {
  return typeof value === "function";
}

function readLifecycleCallback(
  resource: BoundedLifecycleResource,
  property: keyof BoundedLifecycleResource,
): unknown {
  try {
    return resource[property];
  } catch {
    return CALLBACK_READ_FAILED;
  }
}

export class BoundedOwner<T extends NonNullable<unknown>> {
  readonly #limits: Readonly<BoundedOwnerLimits>;
  #queue: (QueuedItem<T> | undefined)[] = [];
  #queueHead = 0;
  #queuedItems = 0;
  readonly #leases = new Set<LeaseRecord>();
  #waiters = new Set<PermitWaiter>();
  #waiterBatch: Set<PermitWaiter> | null = null;
  readonly #resources = new Set<ResourceRecord>();
  readonly #pendingResources = new Set<ResourceRecord>();
  readonly #deferredResources = new Set<ResourceRecord>();
  #state: BoundedOwnerState = "active";
  #queuedBytes = 0;
  #active = 0;
  #waitingPermits = 0;
  #ownedResources = 0;
  #canceled = false;
  #pumping = false;
  #transitioning = false;
  #notifyingRegistration = false;
  #advancingPending = false;
  #resourceCallbackDepth = 0;
  #resourceValidationDepth = 0;
  #validationTrackAllowance = 0;

  constructor(limits: BoundedOwnerLimits) {
    const maxItems = limits.maxItems;
    const maxBytes = limits.maxBytes;
    const maxActive = limits.maxActive;
    if (
      !validLimit(maxItems, HARD_MAX_RESOURCE_ITEMS) ||
      !validLimit(maxBytes, HARD_MAX_RESOURCE_BYTES) ||
      !validLimit(maxActive, HARD_MAX_ACTIVE_PERMITS)
    ) {
      throw new RangeError("bounded_owner_limits");
    }
    this.#limits = Object.freeze({ maxActive, maxBytes, maxItems });
  }

  enqueue(value: T, bytes: number): QueueAdmission {
    if (this.#state === "retired") return "retired";
    if (isNullish(value)) throw new TypeError("bounded_owner_item_value");
    if (!validItemBytes(bytes)) throw new RangeError("bounded_owner_item_bytes");
    if (this.#queuedItems >= this.#limits.maxItems) return "items_exceeded";
    if (bytes > this.#limits.maxBytes - this.#queuedBytes) return "bytes_exceeded";
    this.#queue.push({ bytes, value });
    this.#queuedItems += 1;
    this.#queuedBytes += bytes;
    return "accepted";
  }

  dequeue(): T | null {
    if (this.#transitioning || this.#state !== "active") return null;
    const item = this.#queue[this.#queueHead];
    if (item === undefined) return null;
    this.#queue[this.#queueHead] = undefined;
    this.#queueHead += 1;
    this.#queuedItems -= 1;
    this.#queuedBytes -= item.bytes;
    this.#compactQueue();
    return item.value;
  }

  acquire(): BoundedLease | null {
    this.#pumpWaiters();
    if (
      this.#state !== "active" ||
      this.#transitioning ||
      this.#active >= this.#limits.maxActive ||
      this.#waitingPermits > 0
    ) {
      return null;
    }
    return this.#createLease();
  }

  requestPermit(admit: (lease: BoundedLease) => void): PermitRequest {
    if (typeof admit !== "function") throw new TypeError("bounded_owner_permit_callback");
    this.#pumpWaiters();
    const priorWaitersRemain = this.#waitingPermits > 0;
    const waiter: PermitWaiter = {
      admit,
      lease: null,
      state:
        this.#state === "retired"
          ? "retired"
          : this.#waitingPermits >= this.#limits.maxItems
            ? "items_exceeded"
            : "waiting",
    };
    const request = Object.freeze({
      dispose: () => {
        this.#cancelRequest(waiter);
      },
      state: () => waiter.state,
    });
    if (waiter.state !== "waiting") return request;
    this.#waiters.add(waiter);
    this.#waitingPermits += 1;
    if (!priorWaitersRemain) this.#pumpWaiters();
    return request;
  }

  cancel(): boolean {
    if (this.#canceled) return false;
    this.#canceled = true;
    return true;
  }

  isCanceled(): boolean {
    return this.#canceled;
  }

  track(resource: BoundedLifecycleResource): BoundedDisposable {
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    if (
      this.#state === "active" &&
      this.#resourceValidationDepth === 0 &&
      this.#resourceCallbackDepth === 0 &&
      !this.#transitioning &&
      !this.#notifyingRegistration &&
      !this.#advancingPending &&
      !this.#pumping
    ) {
      this.#advancePendingResources();
    }
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    if (this.#resourceValidationDepth > 0) {
      if (this.#validationTrackAllowance < 1) {
        throw new Error("bounded_owner_resource_reentrant");
      }
      this.#validationTrackAllowance -= 1;
    }
    if (this.#ownedResources >= this.#limits.maxItems) {
      throw new Error("bounded_owner_resource_limit");
    }

    let dispose: unknown;
    let resume: unknown;
    let suspend: unknown;
    this.#resourceValidationDepth += 1;
    try {
      dispose = readLifecycleCallback(resource, "dispose");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
      resume = readLifecycleCallback(resource, "resume");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
      suspend = readLifecycleCallback(resource, "suspend");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    } finally {
      this.#resourceValidationDepth -= 1;
    }
    if (
      dispose === CALLBACK_READ_FAILED ||
      resume === CALLBACK_READ_FAILED ||
      suspend === CALLBACK_READ_FAILED ||
      !isCallback(dispose) ||
      (resume !== undefined && !isCallback(resume)) ||
      (suspend !== undefined && !isCallback(suspend))
    ) {
      throw new TypeError("bounded_owner_resource");
    }
    if (this.#ownedResources >= this.#limits.maxItems) {
      throw new Error("bounded_owner_resource_limit");
    }

    const edgeState = this.#state;
    const record: ResourceRecord = { activated: false, active: true, dispose, resume, suspend };
    this.#resources.add(record);
    this.#pendingResources.add(record);
    this.#ownedResources += 1;
    if (
      this.#transitioning ||
      this.#notifyingRegistration ||
      this.#advancingPending ||
      this.#pumping ||
      this.#resourceCallbackDepth > 0
    ) {
      this.#deferredResources.add(record);
      return Object.freeze({
        dispose: () => {
          this.#disposeResource(record);
        },
      });
    }
    this.#notifyingRegistration = true;
    try {
      if (edgeState === "active") this.#activateResource(record);
    } finally {
      this.#notifyingRegistration = false;
    }
    return Object.freeze({
      dispose: () => {
        this.#disposeResource(record);
      },
    });
  }

  suspend(): BoundedOwnerState {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "active") return this.#state;
    this.#deferredResources.clear();
    this.#state = "suspended";
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (let index = resources.length - 1; index >= 0; index -= 1) {
        const record = resources[index];
        if (record?.active === true && record.activated) {
          this.#invokeResourceCallback(record.suspend);
        }
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("suspended");
    } finally {
      this.#transitioning = false;
    }
    return this.#state;
  }

  resume(): BoundedOwnerState {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "suspended") return this.#state;
    this.#deferredResources.clear();
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (const record of resources) {
        if (record.active) this.#activateResource(record);
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("active");
    } finally {
      this.#transitioning = false;
    }
    if (this.#inState("suspended")) this.#state = "active";
    this.#pumpWaiters();
    return this.#state;
  }

  retire(): BoundedOwnerRetirement {
    if (this.#state === "retired") {
      return Object.freeze({ drainedBytes: 0, drainedItems: 0, releasedPermits: 0 });
    }
    this.#state = "retired";
    this.cancel();

    const drainedItems = this.#queuedItems;
    const drainedBytes = this.#queuedBytes;
    const releasedPermits = this.#active;
    this.#queue = [];
    this.#queueHead = 0;
    this.#queuedItems = 0;
    this.#queuedBytes = 0;

    for (const waiter of this.#waiters) {
      if (waiter.state === "waiting") waiter.state = "retired";
    }
    if (this.#waiterBatch !== null) {
      for (const waiter of this.#waiterBatch) {
        if (waiter.state === "waiting") waiter.state = "retired";
      }
    }
    this.#waiters.clear();
    this.#waiterBatch?.clear();
    this.#waiterBatch = null;
    this.#waitingPermits = 0;
    for (const lease of this.#leases) lease.active = false;
    this.#leases.clear();
    this.#active = 0;

    const resources = [...this.#resources].filter((record) => record.active).reverse();
    this.#resources.clear();
    this.#pendingResources.clear();
    this.#deferredResources.clear();
    for (const record of resources) {
      record.active = false;
      this.#ownedResources -= 1;
    }
    for (const record of resources) this.#invokeResourceCallback(record.dispose);

    return Object.freeze({ drainedBytes, drainedItems, releasedPermits });
  }

  snapshot(): BoundedOwnerSnapshot {
    return Object.freeze({
      active: this.#active,
      canceled: this.#canceled,
      ownedResources: this.#ownedResources,
      pendingResources: this.#pendingResources.size,
      queuedBytes: this.#queuedBytes,
      queuedItems: this.#queuedItems,
      state: this.#state,
      waitingPermits: this.#waitingPermits,
    });
  }

  #createLease(): BoundedLease {
    const record: LeaseRecord = { active: true };
    this.#leases.add(record);
    this.#active += 1;
    return Object.freeze({
      dispose: () => {
        this.#release(record);
      },
    });
  }

  #release(record: LeaseRecord): void {
    if (!record.active) return;
    record.active = false;
    this.#leases.delete(record);
    this.#active -= 1;
    this.#pumpWaiters();
  }

  #cancelRequest(waiter: PermitWaiter): void {
    if (waiter.state === "waiting") {
      waiter.state = "canceled";
      this.#waitingPermits -= 1;
      this.#waiters.delete(waiter);
      this.#waiterBatch?.delete(waiter);
      this.#pumpWaiters();
      return;
    }
    if (waiter.state !== "admitted") return;
    waiter.state = "canceled";
    waiter.lease?.dispose();
  }

  #pumpWaiters(): void {
    if (this.#pumping || this.#state !== "active") return;
    this.#pumping = true;
    const eligible = this.#waiters;
    this.#waiters = new Set<PermitWaiter>();
    this.#waiterBatch = eligible;
    try {
      for (const waiter of eligible) {
        if (this.#active >= this.#limits.maxActive) break;
        eligible.delete(waiter);
        if (waiter.state !== "waiting") continue;
        this.#waitingPermits -= 1;
        const lease = this.#createLease();
        waiter.lease = lease;
        waiter.state = "admitted";
        try {
          waiter.admit(lease);
        } catch {
          this.#cancelRequest(waiter);
        }
        if (!this.#admissionOpen()) break;
      }
    } finally {
      if (!this.#inState("retired")) {
        const additions = this.#waiters;
        for (const waiter of additions) eligible.add(waiter);
        this.#waiters = eligible;
      }
      this.#waiterBatch = null;
      this.#pumping = false;
    }
  }

  #activateResource(record: ResourceRecord): void {
    if (!record.active) return;
    record.activated = true;
    this.#pendingResources.delete(record);
    this.#deferredResources.delete(record);
    this.#invokeResourceCallback(record.resume);
  }

  #disposeResource(record: ResourceRecord): void {
    if (!record.active) return;
    record.active = false;
    this.#resources.delete(record);
    this.#pendingResources.delete(record);
    this.#deferredResources.delete(record);
    this.#ownedResources -= 1;
    this.#invokeResourceCallback(record.dispose);
  }

  #advancePendingResources(): void {
    if (this.#advancingPending || this.#state !== "active") return;
    this.#advancingPending = true;
    const eligible = [...this.#pendingResources];
    try {
      for (const record of eligible) {
        if (!this.#inState("active")) break;
        if (record.active && this.#pendingResources.has(record)) this.#activateResource(record);
      }
    } finally {
      this.#advancingPending = false;
    }
  }

  #invokeResourceCallback(callback: (() => void) | undefined): void {
    const priorAllowance = this.#validationTrackAllowance;
    this.#resourceCallbackDepth += 1;
    if (this.#resourceValidationDepth > 0) this.#validationTrackAllowance = 1;
    try {
      callback?.();
    } catch {
      // A feature callback cannot change resource accounting or prevent later cleanup.
    } finally {
      this.#validationTrackAllowance = priorAllowance;
      this.#resourceCallbackDepth -= 1;
    }
  }

  #compactQueue(): void {
    if (this.#queuedItems === 0) {
      this.#queue = [];
      this.#queueHead = 0;
      return;
    }
    if (this.#queueHead < 1024 || this.#queueHead * 2 < this.#queue.length) return;
    this.#queue = this.#queue.slice(this.#queueHead);
    this.#queueHead = 0;
  }

  #admissionOpen(): boolean {
    return !this.#transitioning && this.#state === "active";
  }

  #inState(state: BoundedOwnerState): boolean {
    return this.#state === state;
  }

  #drainDeferredResources(target: "active" | "suspended"): void {
    const eligible = [...this.#deferredResources];
    this.#deferredResources.clear();
    for (const record of eligible) {
      if (record.active && this.#state !== "retired") {
        if (target === "active") this.#activateResource(record);
        else if (record.activated) this.#invokeResourceCallback(record.suspend);
      }
    }
  }
}
