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
  readonly resource: BoundedLifecycleResource;
  active: boolean;
}

function validLimit(value: number, maximum: number): boolean {
  return Number.isSafeInteger(value) && value >= 1 && value <= maximum;
}

function validItemBytes(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function invoke(callback: (() => void) | undefined): void {
  try {
    callback?.();
  } catch {
    // A feature callback cannot change resource accounting or prevent later cleanup.
  }
}

export class BoundedOwner<T> {
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
    this.#pumpWaiters();
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
    if (
      typeof resource.dispose !== "function" ||
      (resource.resume !== undefined && typeof resource.resume !== "function") ||
      (resource.suspend !== undefined && typeof resource.suspend !== "function")
    ) {
      throw new TypeError("bounded_owner_resource");
    }
    const record: ResourceRecord = { active: true, resource };
    this.#resources.push(record);
    this.#ownedResources += 1;
    if (this.#transitioning) {
      this.#deferredResources.push(record);
      return Object.freeze({
        dispose: () => {
          this.#disposeResource(record);
        },
      });
    }
    if (this.#state === "active") invoke(resource.resume);
    if (this.#state === "suspended") invoke(resource.suspend);
    return Object.freeze({
      dispose: () => {
        this.#disposeResource(record);
      },
    });
  }

  suspend(): BoundedOwnerState {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "active") return this.#state;
    this.#state = "suspended";
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (let index = resources.length - 1; index >= 0; index -= 1) {
        const record = resources[index];
        if (record?.active === true) invoke(record.resource.suspend);
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("suspended");
    } finally {
      this.#deferredResources.length = 0;
      this.#transitioning = false;
    }
    return this.#state;
  }

  resume(): BoundedOwnerState {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "suspended") return this.#state;
    this.#transitioning = true;
    const resources = [...this.#resources];
    try {
      for (const record of resources) {
        if (record.active) invoke(record.resource.resume);
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("active");
    } finally {
      this.#deferredResources.length = 0;
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
    for (const record of resources) invoke(record.resource.dispose);

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
    try {
      while (this.#active < this.#limits.maxActive) {
        const waiter = this.#waiters.shift();
        if (waiter === undefined) break;
        if (waiter.state !== "waiting") continue;
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
    this.#ownedResources -= 1;
    invoke(record.resource.dispose);
  }

  #admissionOpen(): boolean {
    return !this.#transitioning && this.#state === "active";
  }

  #inState(state: BoundedOwnerState): boolean {
    return this.#state === state;
  }

  #drainDeferredResources(target: "active" | "suspended"): void {
    let record = this.#deferredResources.shift();
    while (record !== undefined) {
      if (record.active && this.#state !== "retired") {
        invoke(target === "active" ? record.resource.resume : record.resource.suspend);
      }
      record = this.#deferredResources.shift();
    }
  }
}
