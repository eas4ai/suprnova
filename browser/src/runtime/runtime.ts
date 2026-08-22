import { Idiomorph } from "idiomorph";

import { DocumentRuntime } from "../islands/discovery.js";
import type { JsonValue } from "../canonical.js";
import { RuntimeCallRegistry, type RuntimeCallRegistration } from "../extensions/calls.js";
import {
  EffectRegistry,
  type EffectInvocation,
  type EffectRegistration,
  type EffectRunOutcome,
} from "../extensions/effects.js";
import type { RuntimeConfig } from "./types.js";
import type { RuntimeDiagnostics } from "./diagnostics.js";
import type { RuntimePorts } from "./ports.js";

export type RuntimeStatus = "running" | "stopped";

export interface RuntimeHandle {
  status(): RuntimeStatus;
  stop(): void;
  runEffect(owner: Element, invocation: EffectInvocation): Promise<EffectRunOutcome>;
  call(owner: Element, name: string, input: JsonValue): Promise<JsonValue>;
}

export interface RuntimeContext {
  readonly document: Document;
  readonly config: RuntimeConfig;
  readonly diagnostics: RuntimeDiagnostics;
  readonly ports: RuntimePorts;
  readonly effects?: readonly EffectRegistration[];
  readonly calls?: readonly RuntimeCallRegistration[];
  readonly extensionDeadlineMs?: number;
}

export class SuprnovaLiveRuntime implements RuntimeHandle {
  readonly #context: RuntimeContext;
  readonly #documentRuntime: DocumentRuntime;
  readonly #effects: EffectRegistry;
  readonly #calls: RuntimeCallRegistry;
  #status: RuntimeStatus = "running";

  constructor(context: RuntimeContext) {
    this.#context = context;
    this.#effects = new EffectRegistry({
      diagnostics: context.diagnostics,
      scheduler: context.ports.scheduler,
      ...(context.extensionDeadlineMs === undefined
        ? {}
        : { deadlineMs: context.extensionDeadlineMs }),
    });
    this.#calls = new RuntimeCallRegistry({
      diagnostics: context.diagnostics,
      scheduler: context.ports.scheduler,
      ...(context.extensionDeadlineMs === undefined
        ? {}
        : { deadlineMs: context.extensionDeadlineMs }),
    });
    for (const registration of context.effects ?? []) this.#effects.register(registration);
    for (const registration of context.calls ?? []) this.#calls.register(registration);
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
    this.#effects.dispose();
    this.#calls.dispose();
    this.#status = "stopped";
  }

  runEffect(owner: Element, invocation: EffectInvocation): Promise<EffectRunOutcome> {
    if (this.#status !== "running") return Promise.reject(new Error("runtime_stopped"));
    return this.#documentRuntime.runEffect(this.#effects, this.#calls, owner, invocation);
  }

  call(owner: Element, name: string, input: JsonValue): Promise<JsonValue> {
    if (this.#status !== "running") return Promise.reject(new Error("runtime_stopped"));
    return this.#documentRuntime.call(this.#calls, owner, name, input);
  }

  /** Internal morph seam retained in the shared runtime core for the island pipeline. */
  morph(target: Element | Document, content: Element | Node | string): readonly Node[] | undefined {
    if (this.#status !== "running") throw new Error("runtime_stopped");
    void this.#context;
    return Idiomorph.morph(target, content, { morphStyle: "outerHTML", restoreFocus: false });
  }
}
