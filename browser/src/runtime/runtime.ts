import { Idiomorph } from "idiomorph";

import type { RuntimeConfig } from "./types.js";
import type { RuntimeDiagnostics } from "./diagnostics.js";
import type { RuntimePorts } from "./ports.js";

export type RuntimeStatus = "running" | "stopped";

export interface RuntimeHandle {
  status(): RuntimeStatus;
  stop(): void;
}

export interface RuntimeContext {
  readonly config: RuntimeConfig;
  readonly diagnostics: RuntimeDiagnostics;
  readonly ports: RuntimePorts;
}

export class SuprnovaLiveRuntime implements RuntimeHandle {
  readonly #context: RuntimeContext;
  #status: RuntimeStatus = "running";

  constructor(context: RuntimeContext) {
    this.#context = context;
  }

  status(): RuntimeStatus {
    return this.#status;
  }

  stop(): void {
    this.#status = "stopped";
  }

  /** Internal morph seam retained in the shared runtime core for the island pipeline. */
  morph(target: Element | Document, content: Element | Node | string): readonly Node[] | undefined {
    if (this.#status !== "running") throw new Error("runtime_stopped");
    void this.#context;
    return Idiomorph.morph(target, content, { morphStyle: "outerHTML", restoreFocus: false });
  }
}
