import { EventRouter } from "../directives/events.js";
import { DirectiveOwnership } from "../directives/ownership.js";
import type { JsonValue } from "../canonical.js";
import type { RuntimeCallRegistry } from "../extensions/calls.js";
import type { EffectInvocation, EffectRegistry, EffectRunOutcome } from "../extensions/effects.js";
import type { RuntimeDiagnostics } from "../runtime/diagnostics.js";
import { DelegatedListenerRegistry } from "../runtime/listeners.js";
import type { RuntimePorts } from "../runtime/ports.js";
import type { RuntimeConfig } from "../runtime/types.js";
import { SignalRuntime } from "../signals/lifecycle.js";
import {
  ISLAND_ROOT_SELECTOR,
  ISLAND_STATUS_ATTRIBUTE,
  IslandMetadataError,
  MAX_ISLANDS_PER_DOCUMENT,
  parseIslandMetadata,
} from "./metadata.js";
import { LazyCoordinator } from "./lazy.js";
import { IslandRecord } from "./record.js";

type DocumentRuntimeState = "idle" | "running" | "suspended" | "disposed";

function rootsWithin(node: Node): Element[] {
  if (!(node instanceof Element)) return [];
  const roots = node.matches(ISLAND_ROOT_SELECTOR) ? [node] : [];
  roots.push(...node.querySelectorAll(ISLAND_ROOT_SELECTOR));
  return roots;
}

export class DocumentRuntime {
  readonly #document: Document;
  readonly #config: RuntimeConfig;
  readonly #diagnostics: RuntimeDiagnostics;
  readonly #observer: MutationObserver;
  readonly #listeners: DelegatedListenerRegistry;
  readonly #ownership = new DirectiveOwnership();
  readonly #events: EventRouter;
  readonly #lazy: LazyCoordinator;
  readonly #signals: SignalRuntime;
  readonly #records = new Map<Element, IslandRecord>();
  readonly #identities = new Map<string, IslandRecord>();
  #state: DocumentRuntimeState = "idle";

  constructor(
    document: Document,
    config: RuntimeConfig,
    diagnostics: RuntimeDiagnostics,
    ports: RuntimePorts,
  ) {
    this.#document = document;
    this.#config = config;
    this.#diagnostics = diagnostics;
    this.#listeners = new DelegatedListenerRegistry(document);
    this.#events = new EventRouter(this.#listeners, this.#ownership, ports.randomness, diagnostics);
    this.#signals = new SignalRuntime(this.#events, this.#ownership, ports.scheduler, diagnostics);
    this.#lazy = new LazyCoordinator(ports.observers, ports.randomness);
    this.#observer = ports.observers.mutation((records) => {
      this.#mutations(records);
    });
  }

  start(): void {
    if (this.#state === "disposed") throw new Error("document_runtime_disposed");
    if (this.#state === "running") return;
    this.#state = "running";
    this.#listeners.resume();
    this.#discover(this.#document.querySelectorAll(ISLAND_ROOT_SELECTOR));
    this.#observe();
  }

  suspend(): void {
    if (this.#state !== "running") return;
    this.#observer.disconnect();
    this.#listeners.suspend();
    this.#lazy.suspend();
    this.#state = "suspended";
  }

  resume(): void {
    if (this.#state === "disposed") throw new Error("document_runtime_disposed");
    if (this.#state !== "suspended") return;
    this.#state = "running";
    for (const element of [...this.#records.keys()]) {
      if (!element.isConnected) this.#retire(element);
    }
    this.#listeners.resume();
    this.#lazy.resume();
    this.#discover(this.#document.querySelectorAll(ISLAND_ROOT_SELECTOR));
    this.#observe();
  }

  async runEffect(
    effects: EffectRegistry,
    calls: RuntimeCallRegistry,
    owner: Element,
    invocation: EffectInvocation,
  ): Promise<EffectRunOutcome> {
    const record = this.#ownership.ownerForNode(owner);
    const declared =
      record !== null &&
      record.active() &&
      owner.isConnected &&
      owner === record.element &&
      this.#ownership
        .directives(record)
        .some(
          (candidate) =>
            candidate.element === record.element &&
            candidate.directive.name === "effect" &&
            candidate.directive.value === invocation.name,
        );
    if (!declared) {
      return Object.freeze({ name: invocation.name, status: "invalid_context" });
    }
    const outcomes = await effects.runAll(
      {
        active: () => record.active(),
        island: {
          component: record.metadata.component,
          slot: record.metadata.slot,
          documentKey: record.metadata.documentKey,
        },
        invokeCall: (name, input, active) =>
          this.#invokeDeclaredCall(calls, record, name, input, active),
        phase: "after_commit",
      },
      [invocation],
    );
    return outcomes[0] ?? Object.freeze({ name: invocation.name, status: "invalid_context" });
  }

  call(
    calls: RuntimeCallRegistry,
    owner: Element,
    name: string,
    input: JsonValue,
  ): Promise<JsonValue> {
    const record = this.#ownership.ownerForNode(owner);
    if (record?.active() !== true || !owner.isConnected) {
      return Promise.reject(new Error("extension_context_invalid"));
    }
    const declared = this.#ownership
      .directives(record)
      .find(
        (candidate) =>
          candidate.element === owner &&
          candidate.directive.name === "call" &&
          candidate.directive.value === name,
      );
    if (declared === undefined) return Promise.reject(new Error("extension_context_invalid"));
    return this.#invokeCall(calls, record, declared, name, input, () => true);
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#observer.disconnect();
    this.#listeners.dispose();
    for (const record of this.#records.values()) record.dispose();
    this.#lazy.dispose();
    this.#signals.dispose();
    this.#records.clear();
    this.#identities.clear();
    this.#state = "disposed";
  }

  #observe(): void {
    const root = this.#document.documentElement;
    this.#observer.observe(root, { childList: true, subtree: true });
  }

  #invokeDeclaredCall(
    calls: RuntimeCallRegistry,
    record: IslandRecord,
    name: string,
    input: JsonValue,
    active: () => boolean,
  ): Promise<JsonValue> {
    const declared = this.#ownership
      .directives(record)
      .find(
        (candidate) => candidate.directive.name === "call" && candidate.directive.value === name,
      );
    if (declared === undefined) return Promise.reject(new Error("extension_context_invalid"));
    return this.#invokeCall(calls, record, declared, name, input, active);
  }

  #invokeCall(
    calls: RuntimeCallRegistry,
    record: IslandRecord,
    declared: import("../directives/ownership.js").OwnedDirective,
    name: string,
    input: JsonValue,
    invocationActive: () => boolean,
  ): Promise<JsonValue> {
    return calls.invoke(
      {
        active: () => record.active() && invocationActive(),
        island: {
          component: record.metadata.component,
          slot: record.metadata.slot,
          documentKey: record.metadata.documentKey,
        },
        local: (signal, value) =>
          Promise.resolve(this.#signals.setFromCall(record, declared.element, signal, value)),
        server: (action, value) => {
          if (action !== name || !this.#events.schedulePublicCall(declared, action, value)) {
            return Promise.reject(new Error("scheduler_rejected"));
          }
          return Promise.resolve(value);
        },
      },
      name,
      input,
    );
  }

  #discover(candidates: Iterable<Element>): void {
    for (const element of candidates) this.#connect(element);
  }

  #connect(element: Element): void {
    if (this.#records.has(element)) return;
    if (this.#records.size >= MAX_ISLANDS_PER_DOCUMENT) {
      element.setAttribute(ISLAND_STATUS_ATTRIBUTE, "invalid");
      this.#diagnostics.record({
        code: "resource_limit",
        severity: "error",
        phase: "discovery",
        detailCode: "resource_exhausted",
      });
      return;
    }
    try {
      const metadata = parseIslandMetadata(element, this.#config);
      if (this.#identities.has(metadata.documentKey)) {
        throw new IslandMetadataError("invalid", "duplicate_identity");
      }
      const record = new IslandRecord(element, metadata, this.#config.maxQueuedPerIsland);
      this.#records.set(element, record);
      this.#identities.set(metadata.documentKey, record);
      record.connect();
      const directives = this.#ownership.connect(record);
      this.#signals.connect(record, directives);
      this.#events.connect(record, directives);
      this.#lazy.connect(record, directives);
    } catch (error: unknown) {
      const kind = error instanceof IslandMetadataError ? error.kind : "invalid";
      element.setAttribute(ISLAND_STATUS_ATTRIBUTE, kind);
      this.#diagnostics.record({
        code: "island_invalid",
        severity: "error",
        phase: "discovery",
        detailCode: kind === "incompatible" ? "contract_mismatch" : "invalid_shape",
      });
    }
  }

  #retire(element: Element): void {
    const record = this.#records.get(element);
    if (record === undefined) return;
    this.#records.delete(element);
    this.#identities.delete(record.metadata.documentKey);
    record.dispose();
  }

  #mutations(mutations: readonly MutationRecord[]): void {
    if (this.#state !== "running") return;
    for (const mutation of mutations) {
      for (const removed of mutation.removedNodes) {
        for (const root of rootsWithin(removed)) this.#retire(root);
        const record = this.#ownership.ownerForNode(removed);
        if (record !== null) {
          this.#signals.retireSubtree(record, removed);
          this.#events.retireSubtree(record, removed);
        }
      }
    }
    for (const mutation of mutations) {
      for (const added of mutation.addedNodes) {
        this.#discover(rootsWithin(added));
        const record = this.#ownership.ownerForNode(added);
        if (record !== null) {
          const directives = this.#events.scanInsertion(record, added);
          this.#signals.scanInsertion(record, directives);
          this.#lazy.connect(record, directives);
        }
      }
    }
  }
}
