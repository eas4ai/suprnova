import { ISLAND_STATUS_ATTRIBUTE, type IslandMetadata } from "./metadata.js";
import type { ServerIntent } from "../scheduler/intent.js";

const MAX_DISPOSERS = 64;

export class IslandRecord {
  readonly #disposers: VoidFunction[] = [];
  readonly #intents: ServerIntent[] = [];
  #disposed = false;

  constructor(
    readonly element: Element,
    readonly metadata: IslandMetadata,
    readonly intentCapacity = 8,
  ) {}

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

  enqueue(intent: ServerIntent): boolean {
    if (this.#disposed || this.#intents.length >= this.intentCapacity) return false;
    this.#intents.push(intent);
    intent.onFinish(() => {
      const index = this.#intents.indexOf(intent);
      if (index >= 0) this.#intents.splice(index, 1);
    });
    return true;
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const intent of [...this.#intents]) intent.finish("canceled");
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
