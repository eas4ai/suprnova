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
import { createStimulusMorphBridge } from "../stimulus/bridge.js";
import type { StimulusBootstrapOptions, StimulusMorphBridge } from "../stimulus/port.js";
import { IdiomorphAdapter } from "../morph/idiomorph.js";

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
  readonly stimulus?: StimulusBootstrapOptions;
}

export class SuprnovaLiveRuntime implements RuntimeHandle {
  readonly #documentRuntime: DocumentRuntime;
  readonly #effects: EffectRegistry;
  readonly #calls: RuntimeCallRegistry;
  readonly #stimulus: StimulusMorphBridge | null;
  #status: RuntimeStatus = "running";

  constructor(context: RuntimeContext) {
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
    this.#stimulus =
      context.stimulus === undefined
        ? null
        : createStimulusMorphBridge(context.stimulus, context.diagnostics);
    this.#documentRuntime = new DocumentRuntime(
      context.document,
      context.config,
      context.diagnostics,
      context.ports,
      this.#effects,
      this.#calls,
      new IdiomorphAdapter(),
      this.#stimulus,
    );
    this.#documentRuntime.start();
  }

  status(): RuntimeStatus {
    return this.#status;
  }

  stop(): void {
    if (this.#status === "stopped") return;
    this.#documentRuntime.dispose();
    this.#stimulus?.dispose();
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
}
