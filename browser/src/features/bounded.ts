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

function invoke(callback: (() => void) | undefined): void {
  try {
    callback?.();
  } catch {
    // A feature callback cannot change resource accounting or prevent later cleanup.
  }
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
  readonly #queue: QueuedItem<T>[] = [];
  readonly #leases = new Set<LeaseRecord>();
  readonly #waiters: PermitWaiter[] = [];
  readonly #resources: ResourceRecord[] = [];
  readonly #deferredResources: ResourceRecord[] = [];
  #state: BoundedOwnerState = "active";
  #queuedBytes = 0;
  #active = 0;
  #ownedResources = 0;
  #canceled = false;
  #pumping = false;
  #transitioning = false;
  #notifyingRegistration = false;

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
    if (this.#queue.length >= this.#limits.maxItems) return "items_exceeded";
    if (bytes > this.#limits.maxBytes - this.#queuedBytes) return "bytes_exceeded";
    this.#queue.push({ bytes, value });
    this.#queuedBytes += bytes;
    return "accepted";
  }

  dequeue(): T | null {
    if (this.#transitioning || this.#state !== "active") return null;
    const item = this.#queue.shift();
    if (item === undefined) return null;
    this.#queuedBytes -= item.bytes;
    return item.value;
  }

  acquire(): BoundedLease | null {
    this.#pumpWaiters();
    if (
      this.#state !== "active" ||
      this.#transitioning ||
      this.#active >= this.#limits.maxActive ||
      this.#waiters.some((waiter) => waiter.state === "waiting")
    ) {
      return null;
    }
    return this.#createLease();
  }

  requestPermit(admit: (lease: BoundedLease) => void): PermitRequest {
    if (typeof admit !== "function") throw new TypeError("bounded_owner_permit_callback");
    this.#pumpWaiters();
    const priorWaitersRemain = this.#waiters.length > 0;
    const waiter: PermitWaiter = {
      admit,
      lease: null,
      state:
        this.#state === "retired"
          ? "retired"
          : this.#waiters.length >= this.#limits.maxItems
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
    this.#waiters.push(waiter);
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
    if (this.#state === "retired") throw new Error("bounded_owner_retired");
    if (this.#ownedResources >= this.#limits.maxItems) {
      throw new Error("bounded_owner_resource_limit");
    }

    const dispose = readLifecycleCallback(resource, "dispose");
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    const resume = readLifecycleCallback(resource, "resume");
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    const suspend = readLifecycleCallback(resource, "suspend");
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
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
    const record: ResourceRecord = { active: true, dispose, resume, suspend };
    this.#resources.push(record);
    this.#ownedResources += 1;
    if (this.#transitioning || this.#notifyingRegistration) {
      this.#deferredResources.push(record);
      return Object.freeze({
        dispose: () => {
          this.#disposeResource(record);
        },
      });
    }
    this.#notifyingRegistration = true;
    try {
      if (edgeState === "active") invoke(record.resume);
      else invoke(record.suspend);
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
    this.#deferredResources.length = 0;
    this.#state = "suspended";
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (let index = resources.length - 1; index >= 0; index -= 1) {
        const record = resources[index];
        if (record?.active === true) invoke(record.suspend);
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
    this.#deferredResources.length = 0;
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (const record of resources) {
        if (record.active) invoke(record.resume);
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

    const drainedItems = this.#queue.length;
    const drainedBytes = this.#queuedBytes;
    const releasedPermits = this.#active;
    this.#queue.length = 0;
    this.#queuedBytes = 0;

    for (const waiter of this.#waiters) {
      if (waiter.state === "waiting") waiter.state = "retired";
    }
    this.#waiters.length = 0;
    for (const lease of this.#leases) lease.active = false;
    this.#leases.clear();
    this.#active = 0;

    const resources = this.#resources.filter((record) => record.active).reverse();
    this.#resources.length = 0;
    this.#deferredResources.length = 0;
    for (const record of resources) {
      record.active = false;
      this.#ownedResources -= 1;
    }
    for (const record of resources) invoke(record.dispose);

    return Object.freeze({ drainedBytes, drainedItems, releasedPermits });
  }

  snapshot(): BoundedOwnerSnapshot {
    return Object.freeze({
      active: this.#active,
      canceled: this.#canceled,
      ownedResources: this.#ownedResources,
      queuedBytes: this.#queuedBytes,
      queuedItems: this.#queue.length,
      state: this.#state,
      waitingPermits: this.#waiters.reduce(
        (count, waiter) => count + (waiter.state === "waiting" ? 1 : 0),
        0,
      ),
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
      const index = this.#waiters.indexOf(waiter);
      if (index >= 0) this.#waiters.splice(index, 1);
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
    const eligible = [...this.#waiters];
    try {
      for (const waiter of eligible) {
        if (this.#active >= this.#limits.maxActive) break;
        if (waiter.state !== "waiting") continue;
        const index = this.#waiters.indexOf(waiter);
        if (index < 0) continue;
        this.#waiters.splice(index, 1);
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
      this.#pumping = false;
    }
  }

  #disposeResource(record: ResourceRecord): void {
    if (!record.active) return;
    record.active = false;
    const index = this.#resources.indexOf(record);
    if (index >= 0) this.#resources.splice(index, 1);
    const deferredIndex = this.#deferredResources.indexOf(record);
    if (deferredIndex >= 0) this.#deferredResources.splice(deferredIndex, 1);
    this.#ownedResources -= 1;
    invoke(record.dispose);
  }

  #admissionOpen(): boolean {
    return !this.#transitioning && this.#state === "active";
  }

  #inState(state: BoundedOwnerState): boolean {
    return this.#state === state;
  }

  #drainDeferredResources(target: "active" | "suspended"): void {
    const eligible = this.#deferredResources.splice(0, this.#deferredResources.length);
    for (const record of eligible) {
      if (record.active && this.#state !== "retired") {
        invoke(target === "active" ? record.resume : record.suspend);
      }
    }
  }
}
