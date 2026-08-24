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
import type { RuntimeDiagnosticSink } from "./diagnostics.js";
import type { RuntimePorts } from "./ports.js";
import type { StimulusBootstrapOptions } from "../stimulus/port.js";
import { IdiomorphAdapter } from "../morph/idiomorph.js";
import { BrowserRestoreCompatibility } from "../lifecycle/bfcache.js";
import { DocumentLifecycle } from "../lifecycle/document.js";
import { supportsDocumentFreezeResume } from "../lifecycle/events.js";
import { bindResourceLedger, ResourceLedgerImpl } from "../lifecycle/resources.js";
import type { BootstrapOptions } from "./types.js";
import type {
  RuntimeFeatureDriver,
  RuntimeFeatureDriverRegistrationHost,
  RuntimeFeatureRegistrationOutcome,
} from "../features/host.js";

export type RuntimeStatus = "running" | "stopped" | "suspended";

export interface RuntimeHandle {
  status(): RuntimeStatus;
  stop(): void;
  runEffect(owner: Element, invocation: EffectInvocation): Promise<EffectRunOutcome>;
  call(owner: Element, name: string, input: JsonValue): Promise<JsonValue>;
}

export interface RuntimeContext {
  readonly document: Document;
  readonly config: RuntimeConfig;
  readonly diagnostics: RuntimeDiagnosticSink;
  readonly ports: RuntimePorts;
  readonly effects?: readonly EffectRegistration[];
  readonly calls?: readonly RuntimeCallRegistration[];
  readonly extensionDeadlineMs?: number;
  readonly stimulus?: StimulusBootstrapOptions;
  readonly bootstrapOptions?: BootstrapOptions;
}

export class SuprnovaLiveRuntime implements RuntimeHandle, RuntimeFeatureDriverRegistrationHost {
  readonly #documentRuntime: DocumentRuntime;
  readonly #effects: EffectRegistry;
  readonly #calls: RuntimeCallRegistry;
  readonly #lifecycle: DocumentLifecycle;

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
    this.#documentRuntime = new DocumentRuntime(
      context.document,
      context.config,
      context.diagnostics,
      context.ports,
      this.#effects,
      this.#calls,
      new IdiomorphAdapter(),
      context.stimulus,
    );
    const ledger = new ResourceLedgerImpl();
    bindResourceLedger(this, ledger);
    ledger.add("extension", () => {
      this.#effects.dispose();
    });
    ledger.add("extension", () => {
      this.#calls.dispose();
    });
    ledger.track("controller", {
      dispose: () => {
        this.#documentRuntime.dispose();
      },
      resume: () => {
        this.#documentRuntime.resume();
      },
      suspend: () => {
        this.#documentRuntime.suspend();
      },
    });
    const restore = new BrowserRestoreCompatibility(
      context.document,
      context.config,
      context.bootstrapOptions,
    );
    this.#lifecycle = new DocumentLifecycle({
      compatibility: {
        validate: () => {
          const compatible = restore.validate();
          if (!compatible) {
            context.diagnostics.record({
              code: "lifecycle_notice",
              detailCode: "contract_mismatch",
              phase: "lifecycle",
              severity: "error",
            });
          }
          return compatible;
        },
      },
      document: context.document,
      ledger,
      supportsFreezeResume: supportsDocumentFreezeResume(context.document),
      window: context.document.defaultView ?? context.document,
    });
    this.#documentRuntime.start();
    this.#lifecycle.start();
  }

  status(): RuntimeStatus {
    switch (this.#lifecycle.state()) {
      case "active":
        return "running";
      case "suspended":
      case "restoring":
        return "suspended";
      case "created":
      case "disposed":
        return "stopped";
    }
  }

  stop(): void {
    this.#lifecycle.dispose();
  }

  completeOptionalFeatures(): void {
    this.#documentRuntime.completeOptionalFeatures();
  }

  register(driver: RuntimeFeatureDriver): RuntimeFeatureRegistrationOutcome {
    return this.#documentRuntime.registerFeature(driver);
  }

  runEffect(owner: Element, invocation: EffectInvocation): Promise<EffectRunOutcome> {
    if (this.#lifecycle.state() !== "active") {
      return Promise.reject(
        new Error(this.status() === "stopped" ? "runtime_stopped" : "runtime_suspended"),
      );
    }
    return this.#documentRuntime.runEffect(this.#effects, this.#calls, owner, invocation);
  }

  call(owner: Element, name: string, input: JsonValue): Promise<JsonValue> {
    if (this.#lifecycle.state() !== "active") {
      return Promise.reject(
        new Error(this.status() === "stopped" ? "runtime_stopped" : "runtime_suspended"),
      );
    }
    return this.#documentRuntime.call(this.#calls, owner, name, input);
  }
}
