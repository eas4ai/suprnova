import { Idiomorph } from "idiomorph";

import { DocumentRuntime } from "../islands/discovery.js";
import type { RuntimeConfig } from "./types.js";
import type { RuntimeDiagnostics } from "./diagnostics.js";
import type { RuntimePorts } from "./ports.js";

export type RuntimeStatus = "running" | "stopped";

export interface RuntimeHandle {
  status(): RuntimeStatus;
  stop(): void;
}

export interface RuntimeContext {
  readonly document: Document;
  readonly config: RuntimeConfig;
  readonly diagnostics: RuntimeDiagnostics;
  readonly ports: RuntimePorts;
}

export class SuprnovaLiveRuntime implements RuntimeHandle {
  readonly #context: RuntimeContext;
  readonly #documentRuntime: DocumentRuntime;
  #status: RuntimeStatus = "running";

  constructor(context: RuntimeContext) {
    this.#context = context;
    this.#documentRuntime = new DocumentRuntime(
      context.document,
      context.config,
      context.diagnostics,
      context.ports,
    );
    this.#documentRuntime.start();
  }

  status(): RuntimeStatus {
    return this.#status;
  }

  stop(): void {
    if (this.#status === "stopped") return;
    this.#documentRuntime.dispose();
    this.#status = "stopped";
  }

  /** Internal morph seam retained in the shared runtime core for the island pipeline. */
  morph(target: Element | Document, content: Element | Node | string): readonly Node[] | undefined {
    if (this.#status !== "running") throw new Error("runtime_stopped");
    void this.#context;
    return Idiomorph.morph(target, content, { morphStyle: "outerHTML", restoreFocus: false });
  }
}
