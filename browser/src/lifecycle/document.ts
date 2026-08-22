import {
  type DocumentLifecycleEvent,
  type DocumentLifecycleEventSources,
  normalizeDocumentLifecycleEvent,
} from "./events.js";
import type { ResourceLedger } from "./resources.js";

export type DocumentRuntimeState = "created" | "active" | "suspended" | "restoring" | "disposed";

export interface DocumentLifecycleCompatibility {
  validate(): boolean;
}

export interface DocumentLifecycleOptions extends DocumentLifecycleEventSources {
  readonly compatibility: DocumentLifecycleCompatibility;
  readonly ledger: ResourceLedger;
}

export class DocumentLifecycle {
  readonly #compatibility: DocumentLifecycleCompatibility;
  readonly #events: DocumentLifecycleEventSources;
  readonly #ledger: ResourceLedger;
  #epoch = 0;
  #state: DocumentRuntimeState = "created";

  constructor(options: DocumentLifecycleOptions) {
    this.#compatibility = options.compatibility;
    this.#events = options;
    this.#ledger = options.ledger;
  }

  state(): DocumentRuntimeState {
    return this.#state;
  }

  epoch(): number {
    return this.#epoch;
  }

  start(): void {
    if (this.#state === "disposed") throw new Error("document_lifecycle_disposed");
    if (this.#state !== "created") return;
    this.#listen(this.#events.window, "pagehide");
    this.#listen(this.#events.window, "pageshow");
    if (this.#events.supportsFreezeResume) {
      this.#listen(this.#events.document, "freeze");
      this.#listen(this.#events.document, "resume");
    }
    this.#ledger.resume();
    this.#state = "active";
  }

  suspend(): void {
    if (this.#state !== "active") return;
    this.#ledger.suspend();
    this.#state = "suspended";
  }

  restore(): void {
    if (this.#state !== "suspended") return;
    this.#state = "restoring";
    let compatible: boolean;
    try {
      compatible = this.#compatibility.validate();
    } catch {
      this.dispose();
      return;
    }
    if (!compatible || this.#epoch >= Number.MAX_SAFE_INTEGER) {
      this.dispose();
      return;
    }
    this.#epoch += 1;
    this.#ledger.resume();
    this.#state = "active";
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#ledger.dispose();
  }

  guard<Arguments extends readonly unknown[]>(
    callback: (...arguments_: Arguments) => void,
  ): (...arguments_: Arguments) => void {
    const epoch = this.#epoch;
    return (...arguments_: Arguments): void => {
      if (this.#state !== "active" || this.#epoch !== epoch) return;
      callback(...arguments_);
    };
  }

  readonly #handle = (event: Event): void => {
    const lifecycleEvent = normalizeDocumentLifecycleEvent(event);
    if (lifecycleEvent !== null) this.#apply(lifecycleEvent);
  };

  #apply(event: DocumentLifecycleEvent): void {
    switch (event.kind) {
      case "freeze":
        this.suspend();
        break;
      case "pagehide":
        if (event.persisted) this.suspend();
        else this.dispose();
        break;
      case "pageshow":
        if (event.persisted) this.restore();
        break;
      case "resume":
        this.restore();
        break;
    }
  }

  #listen(target: EventTarget, type: string): void {
    target.addEventListener(type, this.#handle);
    this.#ledger.add("listener", () => {
      target.removeEventListener(type, this.#handle);
    });
  }
}
