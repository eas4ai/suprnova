import { ISLAND_STATUS_ATTRIBUTE, type IslandMetadata } from "./metadata.js";
import type { ServerIntent } from "../scheduler/intent.js";
import { FIFO_POLICY } from "../scheduler/policy.js";
import { IslandScheduler } from "../scheduler/scheduler.js";
import type { SchedulerPolicy } from "../scheduler/types.js";

const MAX_DISPOSERS = 64;

export class IslandRecord {
  readonly #disposers: VoidFunction[] = [];
  readonly scheduler: IslandScheduler;
  #disposed = false;

  constructor(
    readonly element: Element,
    readonly metadata: IslandMetadata,
    readonly intentCapacity = 8,
    readonly parallelCapacity = 1,
  ) {
    this.scheduler = new IslandScheduler({
      maxCompleted: Math.max(64, intentCapacity * 2),
      maxParallel: parallelCapacity,
      maxQueued: intentCapacity,
      maxRecoveries: 3,
    });
  }

  active(): boolean {
    return !this.#disposed;
  }

  connect(): void {
    if (this.#disposed) throw new Error("island_record_disposed");
    this.element.setAttribute(ISLAND_STATUS_ATTRIBUTE, "connected");
  }

  onDispose(disposer: VoidFunction): void {
    if (this.#disposed) {
      disposer();
      return;
    }
    if (this.#disposers.length >= MAX_DISPOSERS) throw new Error("island_disposal_limit");
    this.#disposers.push(disposer);
  }

  enqueue(intent: ServerIntent, policy: SchedulerPolicy = FIFO_POLICY): boolean {
    if (this.#disposed) return false;
    return this.scheduler.schedule(intent, policy).disposition === "accepted";
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.scheduler.retire();
    for (let index = this.#disposers.length - 1; index >= 0; index -= 1) {
      try {
        this.#disposers[index]?.();
      } catch {
        // Disposal is best-effort but remains exactly-once for every registered resource.
      }
    }
    this.#disposers.length = 0;
    this.element.setAttribute(ISLAND_STATUS_ATTRIBUTE, "disconnected");
  }
}
