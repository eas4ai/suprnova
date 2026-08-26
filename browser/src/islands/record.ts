import { ISLAND_STATUS_ATTRIBUTE, type IslandMetadata } from "./metadata.js";
import { createFreshRenderIntent, type ServerIntent } from "../scheduler/intent.js";
import { FIFO_POLICY } from "../scheduler/policy.js";
import { IslandScheduler } from "../scheduler/scheduler.js";
import type { SchedulerPolicy } from "../scheduler/types.js";
import type {
  FreshRenderCompletion,
  FreshRenderCompletionObserver,
  FreshRenderDisposition,
  FreshRenderReason,
} from "../features/host.js";

const MAX_DISPOSERS = 64;
const FEATURE_REFRESH_POLICY = Object.freeze({
  key: "feature-fresh-render",
  kind: "replace_pending",
} as const);

export class IslandRecord {
  readonly #disposers: VoidFunction[] = [];
  readonly scheduler: IslandScheduler;
  readonly element: Element;
  #metadata: IslandMetadata;
  #scheduleObserver: VoidFunction | null = null;
  #disposed = false;
  #connectionEpoch = 0;

  constructor(
    element: Element,
    metadata: IslandMetadata,
    readonly intentCapacity = 8,
    readonly parallelCapacity = 1,
  ) {
    this.element = element;
    this.#metadata = metadata;
    this.scheduler = new IslandScheduler({
      maxCompleted: Math.max(64, intentCapacity * 2),
      maxParallel: parallelCapacity,
      maxQueued: intentCapacity,
      maxRecoveries: 1,
    });
  }

  active(): boolean {
    return !this.#disposed;
  }

  get metadata(): IslandMetadata {
    return this.#metadata;
  }

  commitMetadata(metadata: IslandMetadata): void {
    if (this.#disposed) throw new Error("island_record_disposed");
    this.#metadata = metadata;
  }

  connect(): void {
    if (this.#disposed) throw new Error("island_record_disposed");
    this.#connectionEpoch += 1;
    this.element.setAttribute(ISLAND_STATUS_ATTRIBUTE, "connected");
  }

  connectionEpoch(): number {
    return this.#connectionEpoch;
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
    const accepted = this.scheduler.schedule(intent, policy).disposition === "accepted";
    if (accepted) {
      try {
        this.#scheduleObserver?.();
      } catch {
        // Transport wakeup cannot rewrite the already accepted scheduler disposition.
      }
    }
    return accepted;
  }

  enqueueFreshRender(
    reason: FreshRenderReason,
    completion?: FreshRenderCompletionObserver,
  ): FreshRenderDisposition {
    if (this.#disposed) {
      notifyFreshRenderCompletion(completion, "retired");
      return "retired";
    }
    const intent = createFreshRenderIntent(this, reason);
    if (completion !== undefined) {
      intent.onFinish((finish) => {
        const result: FreshRenderCompletion =
          finish === "accepted"
            ? "succeeded"
            : finish === "retired"
              ? "retired"
              : finish === "canceled"
                ? "canceled"
                : "failed";
        notifyFreshRenderCompletion(completion, result);
      });
    }
    const result = this.scheduler.schedule(intent, FEATURE_REFRESH_POLICY);
    if (result.disposition !== "accepted") {
      return result.disposition === "retired" ? "retired" : "coalesced";
    }
    try {
      this.#scheduleObserver?.();
    } catch {
      // Transport wakeup cannot rewrite the already accepted scheduler disposition.
    }
    return "queued";
  }

  attachScheduleObserver(observer: VoidFunction): void {
    if (this.#disposed || this.#scheduleObserver !== null) {
      throw new Error("island_schedule_observer_rejected");
    }
    this.#scheduleObserver = observer;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#scheduleObserver = null;
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

function notifyFreshRenderCompletion(
  observer: FreshRenderCompletionObserver | undefined,
  completion: FreshRenderCompletion,
): void {
  if (observer === undefined) return;
  try {
    observer(completion);
  } catch {
    // Completion presentation cannot rewrite scheduler authority.
  }
}
