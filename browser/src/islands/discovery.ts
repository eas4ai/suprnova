import { EventRouter } from "../directives/events.js";
import { DirectiveOwnership } from "../directives/ownership.js";
import { canonicalize, type JsonValue } from "../canonical.js";
import { queueChildDeliveries } from "../application/children.js";
import { dispatchValidatedEvents, runValidatedEffects } from "../application/emissions.js";
import { ResponseApplicationMachine, type ApplicationPorts } from "../application/machine.js";
import type { ValidatedCommittedResponse } from "../application/types.js";
import { applyUrlReflection } from "../application/url.js";
import { FeedbackRuntime } from "../feedback/targets.js";
import type { RuntimeCallRegistry } from "../extensions/calls.js";
import type { EffectInvocation, EffectRegistry, EffectRunOutcome } from "../extensions/effects.js";
import type { RuntimeDiagnostics } from "../runtime/diagnostics.js";
import { DelegatedListenerRegistry } from "../runtime/listeners.js";
import type { RuntimePorts } from "../runtime/ports.js";
import type { RuntimeConfig } from "../runtime/types.js";
import type { SchedulerTicket } from "../scheduler/types.js";
import { createFreshRenderIntent, createParamsChangedIntent } from "../scheduler/intent.js";
import { SignalRuntime } from "../signals/lifecycle.js";
import type { StimulusMorphBridge } from "../stimulus/port.js";
import { LiveTransportCoordinator } from "../transport/fetch.js";
import { parseUpdateResponse } from "../protocol.js";
import { DEFAULT_MORPH_LIMITS } from "../morph/limits.js";
import { preflightIslandMorph } from "../morph/preflight.js";
import { consumeMorphProvenance } from "../morph/idiomorph.js";
import { TeleportRegistry } from "../morph/teleport.js";
import type { MorphAdapter, MorphPlan } from "../morph/types.js";
import {
  ISLAND_ROOT_SELECTOR,
  ISLAND_STATUS_ATTRIBUTE,
  IslandMetadataError,
  MAX_ISLANDS_PER_DOCUMENT,
  parseIslandMetadata,
  type IslandMetadata,
} from "./metadata.js";
import { LazyCoordinator } from "./lazy.js";
import { IslandRecord } from "./record.js";

type DocumentRuntimeState = "idle" | "running" | "suspended" | "disposed";

interface PreparedSuccessor {
  readonly metadata: IslandMetadata;
  readonly plan: MorphPlan;
}

function rootsWithin(node: Node): Element[] {
  if (!(node instanceof Element)) return [];
  const roots = node.matches(ISLAND_ROOT_SELECTOR) ? [node] : [];
  roots.push(...node.querySelectorAll(ISLAND_ROOT_SELECTOR));
  return roots;
}

function base64UrlText(value: string): string {
  let binary = "";
  for (const byte of new TextEncoder().encode(value)) binary += String.fromCodePoint(byte);
  return btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
}

export class DocumentRuntime {
  readonly #document: Document;
  readonly #config: RuntimeConfig;
  readonly #diagnostics: RuntimeDiagnostics;
  readonly #observer: MutationObserver;
  readonly #listeners: DelegatedListenerRegistry;
  readonly #ownership = new DirectiveOwnership();
  readonly #events: EventRouter;
  readonly #feedback: FeedbackRuntime;
  readonly #lazy: LazyCoordinator;
  readonly #signals: SignalRuntime;
  readonly #transport: LiveTransportCoordinator;
  readonly #ports: RuntimePorts;
  readonly #effects: EffectRegistry;
  readonly #calls: RuntimeCallRegistry;
  readonly #morph: MorphAdapter;
  readonly #teleports: TeleportRegistry;
  readonly #stimulus: StimulusMorphBridge | null;
  readonly #records = new Map<Element, IslandRecord>();
  readonly #identities = new Map<string, IslandRecord>();
  readonly #childParameterHashes = new WeakMap<IslandRecord, string[]>();
  #state: DocumentRuntimeState = "idle";

  constructor(
    document: Document,
    config: RuntimeConfig,
    diagnostics: RuntimeDiagnostics,
    ports: RuntimePorts,
    effects: EffectRegistry,
    calls: RuntimeCallRegistry,
    morph: MorphAdapter,
    stimulus: StimulusMorphBridge | null = null,
  ) {
    this.#document = document;
    this.#config = config;
    this.#diagnostics = diagnostics;
    this.#ports = ports;
    this.#effects = effects;
    this.#calls = calls;
    this.#morph = morph;
    this.#teleports = new TeleportRegistry(document);
    this.#listeners = new DelegatedListenerRegistry(document);
    this.#events = new EventRouter(
      this.#listeners,
      this.#ownership,
      ports.randomness,
      ports.clock,
      ports.scheduler,
      diagnostics,
    );
    this.#signals = new SignalRuntime(this.#events, this.#ownership, ports.scheduler, diagnostics);
    this.#feedback = new FeedbackRuntime(ports.clock, ports.scheduler);
    this.#transport = new LiveTransportCoordinator(config, ports, diagnostics, (record, ticket) => {
      void this.#applyResponse(record, ticket);
    });
    this.#stimulus = stimulus;
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
    this.#observe();
    this.#discover(this.#document.querySelectorAll(ISLAND_ROOT_SELECTOR));
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
    this.#observe();
    this.#discover(this.#document.querySelectorAll(ISLAND_ROOT_SELECTOR));
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
    this.#transport.dispose();
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

  async #applyResponse(record: IslandRecord, ticket: SchedulerTicket): Promise<void> {
    const completed = this.#transport.takeResponse(record, ticket);
    if (completed === null) return;
    try {
      const response = parseUpdateResponse(completed.response.text);
      const applicationDisposition = record.scheduler.beginApplication(ticket);
      const machine = new ResponseApplicationMachine(this.#applicationPorts(record));
      const result = await machine.apply(
        response,
        Object.freeze({
          active: record.active(),
          component: record.metadata.component,
          connectionEpoch: record.connectionEpoch(),
          documentKey: record.metadata.documentKey,
          instanceId: record.metadata.instanceId,
          revision: record.metadata.revision,
          slot: record.metadata.slot,
          snapshotForm: record.metadata.snapshotForm,
        }),
        Object.freeze({
          applicationDisposition,
          baseRevision: completed.request.identity.baseRevision,
          connectionEpoch: record.connectionEpoch(),
          correlationId: completed.request.identity.correlationId,
          promotion: completed.request.identity.promotionNonce !== null,
          protocol: completed.request.protocolVersion,
        }),
      );
      const accepted = result.disposition === "committed" || result.disposition === "navigated";
      record.scheduler.finish(ticket, accepted ? "accepted" : "rejected");
    } catch {
      record.scheduler.finish(ticket, "rejected");
      this.#diagnostics.record({
        code: "response_invalid",
        detailCode: "invalid_response",
        phase: "response",
        severity: "error",
      });
    } finally {
      this.#transport.resume(record);
    }
  }

  #applicationPorts(record: IslandRecord): ApplicationPorts<PreparedSuccessor> {
    let successorMetadata: IslandMetadata | null = null;
    let noRenderSnapshot: string | null = null;
    const requestFreshRender = (): void => {
      try {
        const intent = createFreshRenderIntent(record);
        if (!record.enqueue(intent)) intent.finish("rejected");
      } catch {
        this.#ports.navigation.reload();
      }
    };
    return {
      commit: (response) => {
        if (successorMetadata === null) throw new Error("successor_metadata_missing");
        this.#assertSuccessorMetadata(record, response, successorMetadata);
        if (noRenderSnapshot !== null) {
          record.element.setAttribute("data-suprnova-live-snapshot", noRenderSnapshot);
          record.element.setAttribute("data-suprnova-live-snapshot-kind", "instance");
          record.element.setAttribute(
            "data-suprnova-live-revision",
            response.acceptedRevision.toString(10),
          );
          record.element.setAttribute(
            "data-suprnova-live-instance",
            response.snapshotView.instanceId ?? "",
          );
        }
        record.commitMetadata(successorMetadata);
      },
      dispatchEvents: (response) => {
        dispatchValidatedEvents(response.events, {
          dispatch: (emission) => {
            const window = this.#document.defaultView;
            if (window === null) throw new Error("runtime_window_unavailable");
            record.element.dispatchEvent(
              new window.CustomEvent(`suprnova:${emission.name}`, {
                bubbles: true,
                composed: false,
                detail: emission.payload,
              }),
            );
          },
        });
      },
      morph: (prepared) => {
        const continuity = this.#stimulus?.beforeMorph(record.element) ?? null;
        let teleport: ReturnType<TeleportRegistry["begin"]> | null = null;
        try {
          teleport = this.#teleports.begin(prepared.plan);
          const result = this.#morph.apply(prepared.plan, {});
          this.#teleports.commit(teleport, result.root);
          if (continuity !== null) this.#stimulus?.afterMorph(continuity, result.root);
          const metadata = parseIslandMetadata(result.root, this.#config);
          this.#assertSameMetadata(prepared.metadata, metadata);
          successorMetadata = metadata;
        } catch (error: unknown) {
          if (teleport?.active === true) this.#teleports.rollback(teleport);
          this.#stimulus?.disposeScope(record.element);
          throw error;
        }
      },
      navigate: (response) => {
        if (response.kind === "navigation") {
          this.#ports.navigation.assign(new URL(response.target, this.#document.baseURI));
        } else {
          this.#ports.navigation.reload();
        }
      },
      postCommitFailure: () => {
        this.#diagnostics.record({
          code: "response_invalid",
          detailCode: "recovery_required",
          phase: "response",
          severity: "error",
        });
        requestFreshRender();
      },
      preflight: (response) => this.#preflightSuccessor(record, response),
      queueChildren: (response) => {
        queueChildDeliveries(response.childDeliveries, {
          find: (instanceId) => {
            const child = [...this.#records.values()].find(
              (candidate) => candidate.metadata.instanceId === instanceId,
            );
            if (child === undefined) return null;
            return {
              active: () => child.active(),
              instanceId,
              queueParamsChanged: (envelope, parameterHash) =>
                this.#queueChildParameters(child, envelope, parameterHash),
            };
          },
        });
      },
      reconcile: (response) => {
        const models = this.#events.modelState(record);
        if (models === null) return;
        for (const field of models.fields()) {
          const raw = response.validation[field];
          const messages =
            typeof raw === "string"
              ? [raw]
              : Array.isArray(raw)
                ? raw.filter((value): value is string => typeof value === "string")
                : [];
          models.setValidation(
            field,
            messages.map((message) => Object.freeze({ message })),
          );
        }
      },
      reflectUrl: (response) => {
        if (response.reflectedUrl === null) return;
        const window = this.#document.defaultView;
        if (window === null) throw new Error("runtime_window_unavailable");
        applyUrlReflection(new URL(window.location.href), response.reflectedUrl, (target) => {
          this.#ports.navigation.replace(target);
        });
      },
      requestFreshIsland: () => {
        this.#ports.navigation.reload();
      },
      requestFreshRender,
      restoreFocus: () => undefined,
      retainDom: () => undefined,
      runEffects: (response) =>
        runValidatedEffects(response.effects, {
          effect: async (emission) => {
            await this.#effects.runAll(
              {
                active: () => record.active(),
                island: {
                  component: record.metadata.component,
                  documentKey: record.metadata.documentKey,
                  slot: record.metadata.slot,
                },
                invokeCall: (name, input, active) =>
                  this.#invokeDeclaredCall(this.#calls, record, name, input, active),
                phase: "after_commit",
              },
              [emission],
            );
          },
        }),
      settleFeedback: () => undefined,
      stopLive: () => {
        this.#retire(record.element);
      },
      validateNoRender: (response) => {
        const successor = this.#metadataFromResponse(record, response);
        successorMetadata = successor.metadata;
        noRenderSnapshot = successor.encodedSnapshot;
      },
    };
  }

  #preflightSuccessor(
    record: IslandRecord,
    response: ValidatedCommittedResponse,
  ): PreparedSuccessor {
    if (response.render.kind !== "html") throw new Error("successor_render_missing");
    if (record.metadata.instanceId === null) throw new Error("successor_instance_missing");
    const currentRoot = record.element as HTMLElement;
    const plan = preflightIslandMorph({
      authority: {
        component: record.metadata.component,
        documentKey: record.metadata.documentKey,
        encodedSnapshot: base64UrlText(canonicalize(response.snapshot)),
        instanceId: response.snapshotView.instanceId ?? record.metadata.instanceId,
        slot: record.metadata.slot,
        successorRevision: response.acceptedRevision,
      },
      currentRoot,
      html: response.render.html,
      limits: DEFAULT_MORPH_LIMITS,
      teleports: this.#teleports,
    });
    const metadata = parseIslandMetadata(plan.replacementRoot, this.#config);
    this.#assertSuccessorMetadata(record, response, metadata);
    return Object.freeze({ metadata, plan });
  }

  #metadataFromResponse(
    record: IslandRecord,
    response: ValidatedCommittedResponse,
  ): Readonly<{ metadata: IslandMetadata; encodedSnapshot: string }> {
    const encoded = base64UrlText(canonicalize(response.snapshot));
    const candidate = record.element.cloneNode(false);
    if (!(candidate instanceof Element)) throw new Error("successor_root_invalid");
    candidate.setAttribute("data-suprnova-live-snapshot", encoded);
    candidate.setAttribute("data-suprnova-live-snapshot-kind", "instance");
    candidate.setAttribute("data-suprnova-live-revision", response.acceptedRevision.toString(10));
    candidate.setAttribute("data-suprnova-live-instance", response.snapshotView.instanceId ?? "");
    const metadata = parseIslandMetadata(candidate, this.#config);
    this.#assertSuccessorMetadata(record, response, metadata);
    return Object.freeze({ encodedSnapshot: encoded, metadata });
  }

  #assertSuccessorMetadata(
    record: IslandRecord,
    response: ValidatedCommittedResponse,
    metadata: IslandMetadata,
  ): void {
    if (
      metadata.component !== record.metadata.component ||
      metadata.slot !== record.metadata.slot ||
      metadata.documentKey !== record.metadata.documentKey ||
      metadata.snapshotForm !== "instance" ||
      metadata.instanceId !== response.snapshotView.instanceId ||
      metadata.revision !== response.acceptedRevision ||
      canonicalize(metadata.snapshot as JsonValue) !== canonicalize(response.snapshot)
    ) {
      throw new Error("successor_metadata_disagreement");
    }
  }

  #assertSameMetadata(expected: IslandMetadata, actual: IslandMetadata): void {
    if (
      expected.component !== actual.component ||
      expected.slot !== actual.slot ||
      expected.documentKey !== actual.documentKey ||
      expected.instanceId !== actual.instanceId ||
      expected.revision !== actual.revision ||
      canonicalize(expected.snapshot as JsonValue) !== canonicalize(actual.snapshot as JsonValue)
    ) {
      throw new Error("morph_metadata_disagreement");
    }
  }

  #queueChildParameters(
    child: IslandRecord,
    envelope: Readonly<Record<string, JsonValue>>,
    parameterHash: string,
  ): boolean {
    const hashes = this.#childParameterHashes.get(child) ?? [];
    if (hashes.includes(parameterHash)) return false;
    const intent = createParamsChangedIntent(child, envelope);
    if (!child.enqueue(intent)) {
      intent.finish("rejected");
      return false;
    }
    hashes.push(parameterHash);
    if (hashes.length > 64) hashes.shift();
    this.#childParameterHashes.set(child, hashes);
    return true;
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
      const record = new IslandRecord(
        element,
        metadata,
        this.#config.maxQueuedPerIsland,
        this.#config.maxParallelPerIsland,
      );
      this.#records.set(element, record);
      this.#identities.set(metadata.documentKey, record);
      record.connect();
      this.#transport.connect(record);
      record.onDispose(() => {
        this.#stimulus?.disposeScope(element);
        this.#teleports.disposeOwner(element);
      });
      const directives = this.#ownership.connect(record);
      this.#signals.connect(record, directives);
      this.#events.connect(record, directives);
      this.#feedback.connect(record, directives, this.#events.modelState(record));
      this.#lazy.connect(record, directives);
      this.#teleports.mount(record.element);
    } catch (error: unknown) {
      this.#retire(element);
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
        if (this.#teleports.consumeControlledMove(removed)) continue;
        for (const root of rootsWithin(removed)) this.#retire(root);
        const record = this.#ownership.ownerForNode(removed);
        if (record !== null) {
          this.#signals.retireSubtree(record, removed);
          this.#feedback.retireSubtree(record, removed);
          this.#events.retireSubtree(record, removed);
        }
      }
    }
    for (const mutation of mutations) {
      for (const added of mutation.addedNodes) {
        if (this.#teleports.consumeControlledMove(added)) continue;
        const existingOwner = this.#ownership.ownerForNode(added);
        if (existingOwner !== null && !consumeMorphProvenance(added)) continue;
        this.#discover(rootsWithin(added));
        const record = this.#ownership.ownerForNode(added);
        if (record !== null) {
          const directives = this.#events.scanInsertion(record, added, true);
          this.#signals.scanInsertion(record, directives);
          this.#feedback.scanInsertion(record, directives);
          this.#lazy.connect(record, directives);
        }
      }
    }
  }
}
