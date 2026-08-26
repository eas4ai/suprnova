import { EventRouter } from "../directives/events.js";
import { DirectiveOwnership } from "../directives/ownership.js";
import { canonicalize, type JsonValue } from "../canonical.js";
import { queueChildDeliveries } from "../application/children.js";
import { dispatchValidatedEvents, runValidatedEffects } from "../application/emissions.js";
import { ResponseApplicationMachine, type ApplicationPorts } from "../application/machine.js";
import { ApplicationRecovery, type RecoveryDecision } from "../application/recovery.js";
import type { ValidatedCommittedResponse } from "../application/types.js";
import { applyUrlReflection } from "../application/url.js";
import { FeedbackRuntime } from "../feedback/targets.js";
import type { RuntimeCallRegistry } from "../extensions/calls.js";
import type { EffectInvocation, EffectRegistry, EffectRunOutcome } from "../extensions/effects.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import { DelegatedListenerRegistry } from "../runtime/listeners.js";
import type { RuntimePorts } from "../runtime/ports.js";
import type { RuntimeConfig } from "../runtime/types.js";
import type { SchedulerTicket } from "../scheduler/types.js";
import { createFreshRenderIntent, createParamsChangedIntent } from "../scheduler/intent.js";
import { SignalRuntime } from "../signals/lifecycle.js";
import type { StimulusBootstrapOptions } from "../stimulus/port.js";
import { LiveTransportCoordinator } from "../transport/fetch.js";
import { parseUpdateResponse } from "../protocol.js";
import { captureContinuity, CompositionTracker } from "../continuity/capture.js";
import { restoreContinuity, restoreContinuityFocus } from "../continuity/restore.js";
import type { ContinuityRecord } from "../continuity/types.js";
import { DEFAULT_MORPH_LIMITS } from "../morph/limits.js";
import { preflightIslandMorph } from "../morph/preflight.js";
import { consumeMorphProvenance } from "../morph/idiomorph.js";
import { TeleportRegistry } from "../morph/teleport.js";
import type { MorphAdapter, MorphPlan } from "../morph/types.js";
import {
  BrowserTransitionCompletion,
  prepareMorphTransitions,
  TransitionLifecycle,
} from "../transitions/lifecycle.js";
import { TransitionRunner } from "../transitions/runner.js";
import { NativeDocumentNavigation } from "../navigation/native.js";
import { DIRTY_WORK_ATTRIBUTE, NAVIGATION_GUARD_ATTRIBUTE } from "../navigation/guards.js";
import {
  type UploadHandleProposal,
  type UploadHandleProposalDisposition,
} from "../uploads/types.js";
import { declaresUploadField, UploadProposalAuthority } from "../uploads/proposal.js";
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
import {
  RegisteredEventAuthority,
  type GuardedRegisteredEventTarget,
} from "./registered-events.js";
import {
  inspectRuntimeFeatureDriver,
  type FreshRenderCompletionObserver,
  type FreshRenderReason,
  type InspectedRuntimeFeatureDriver,
  type RuntimeFeatureDiagnosticDetail,
  type RuntimeFeatureDriver,
  type RuntimeFeatureDriverDocumentPort,
  type RuntimeFeatureDriverIslandPort,
  type RuntimeFeatureDriverValue,
  type RuntimeFeatureRegistrationOutcome,
  type RegisteredBrowserEventCapability,
  type RegisteredBrowserEventDispatch,
  type RegisteredBrowserEventDisposition,
  type RegisteredBrowserEventRegistration,
} from "../features/host.js";

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
  readonly #diagnostics: RuntimeDiagnosticSink;
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
  readonly #stimulus: StimulusBootstrapOptions | undefined;
  readonly #composition: CompositionTracker;
  readonly #records = new Map<Element, IslandRecord>();
  readonly #registeredEvents = new RegisteredEventAuthority();
  readonly #featureDriverClaims = new WeakSet<IslandRecord>();
  readonly #identities = new Map<string, IslandRecord>();
  readonly #childParameterHashes = new WeakMap<IslandRecord, string[]>();
  readonly #recoveries = new WeakMap<IslandRecord, ApplicationRecovery>();
  readonly #uploadProposals = new UploadProposalAuthority<IslandRecord>();
  readonly #transitions = new WeakMap<IslandRecord, TransitionLifecycle>();
  readonly #transitionCompletion = new BrowserTransitionCompletion();
  readonly #navigation: NativeDocumentNavigation;
  #featureDriver: InspectedRuntimeFeatureDriver | null = null;
  #featureDriverState: 0 | 1 | 2 = 0;
  #state: DocumentRuntimeState = "idle";

  constructor(
    document: Document,
    config: RuntimeConfig,
    diagnostics: RuntimeDiagnosticSink,
    ports: RuntimePorts,
    effects: EffectRegistry,
    calls: RuntimeCallRegistry,
    morph: MorphAdapter,
    stimulus?: StimulusBootstrapOptions,
  ) {
    this.#document = document;
    this.#config = config;
    this.#diagnostics = diagnostics;
    this.#ports = ports;
    this.#effects = effects;
    this.#calls = calls;
    this.#morph = morph;
    this.#teleports = new TeleportRegistry(document);
    this.#composition = new CompositionTracker(document);
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
    this.#navigation = new NativeDocumentNavigation(document, ports);
  }

  start(): void {
    if (this.#state === "disposed") throw new Error("document_runtime_disposed");
    if (this.#state === "running") return;
    this.#state = "running";
    this.#startFeatureDriver();
    this.#listeners.resume();
    this.#navigation.start();
    this.#observe();
    this.#discover(this.#document.querySelectorAll(ISLAND_ROOT_SELECTOR));
  }

  suspend(): void {
    if (this.#state !== "running") return;
    this.#state = "suspended";
    this.#observer.disconnect();
    this.#driveFeatureDriver(2, null);
    this.#listeners.suspend();
    this.#events.suspend();
    this.#feedback.suspend();
    for (const record of this.#records.values()) this.#transitions.get(record)?.cancel("canceled");
    this.#navigation.suspend();
    this.#lazy.suspend();
    this.#transport.suspend();
  }

  resume(): void {
    if (this.#state === "disposed") throw new Error("document_runtime_disposed");
    if (this.#state !== "suspended") return;
    this.#state = "running";
    for (const element of [...this.#records.keys()]) {
      if (!element.isConnected) this.#retire(element);
      else this.#records.get(element)?.connect();
    }
    this.#listeners.resume();
    this.#navigation.resume();
    this.#lazy.resume();
    this.#transport.restore();
    this.#driveFeatureDriver(3, null);
    this.#startFeatureDriver();
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
    const connectionEpoch = record.connectionEpoch();
    const outcomes = await effects.runAll(
      {
        active: () => this.#current(record, connectionEpoch),
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
    const connectionEpoch = record.connectionEpoch();
    return this.#invokeCall(calls, record, declared, name, input, () =>
      this.#current(record, connectionEpoch),
    );
  }

  dispose(): void {
    if (this.#state === "disposed") return;
    this.#state = "disposed";
    this.#observer.disconnect();
    this.#listeners.dispose();
    this.#navigation.dispose();
    for (const record of this.#records.values()) record.dispose();
    const driver = this.#featureDriver;
    const started = this.#featureDriverState !== 0;
    this.#featureDriver = null;
    this.#featureDriverState = 0;
    if (driver !== null && started) this.#invokeFeatureDriver(driver, 5, null);
    this.#transport.dispose();
    this.#lazy.dispose();
    this.#signals.dispose();
    this.#composition.dispose();
    this.#uploadProposals.dispose();
    this.#records.clear();
    this.#identities.clear();
  }

  registerFeature(driver: RuntimeFeatureDriver): RuntimeFeatureRegistrationOutcome {
    if (this.#state === "disposed") return "incompatible";
    const inspected = inspectRuntimeFeatureDriver(driver);
    if (inspected === null) {
      this.#featureDiagnostic("contract_mismatch");
      return "incompatible";
    }
    if (this.#featureDriver !== null) {
      return this.#featureDriver === driver ? "already_registered" : "conflict";
    }
    this.#featureDriver = inspected;
    if (this.#state === "running") this.#startFeatureDriver();
    return "registered";
  }

  completeOptionalFeatures(): void {
    if (this.#stimulus !== undefined && this.#featureDriverState !== 2) {
      this.#featureDiagnostic("contract_mismatch");
    }
  }

  #observe(): void {
    const root = this.#document.documentElement;
    this.#observer.observe(root, {
      attributeFilter: [DIRTY_WORK_ATTRIBUTE, NAVIGATION_GUARD_ATTRIBUTE],
      attributes: true,
      childList: true,
      subtree: true,
    });
  }

  #current(record: IslandRecord, connectionEpoch: number): boolean {
    return (
      this.#state === "running" && record.active() && record.connectionEpoch() === connectionEpoch
    );
  }

  async #applyResponse(record: IslandRecord, ticket: SchedulerTicket): Promise<void> {
    const completed = this.#transport.takeResponse(record, ticket);
    if (completed === null) return;
    if (this.#state !== "running" || completed.connectionEpoch !== record.connectionEpoch()) {
      record.scheduler.finish(ticket, "rejected");
      return;
    }
    try {
      const response = parseUpdateResponse(completed.response.text);
      const applicationDisposition = record.scheduler.beginApplication(ticket);
      const machine = new ResponseApplicationMachine(
        this.#applicationPorts(record, completed.connectionEpoch),
      );
      const result = await machine.apply(
        response,
        Object.freeze({
          active: record.active(),
          component: record.metadata.component,
          connectionEpoch: completed.connectionEpoch,
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
      if (this.#running()) this.#transport.resume(record);
    }
  }

  #applicationPorts(
    record: IslandRecord,
    connectionEpoch: number,
  ): ApplicationPorts<PreparedSuccessor> {
    const continuityRoot = record.element;
    if (!(continuityRoot instanceof HTMLElement)) throw new Error("island_root_invalid");
    let successorMetadata: IslandMetadata | null = null;
    let noRenderSnapshot: string | null = null;
    let continuity: ContinuityRecord | null = null;
    let featureMorphActive = false;
    const recovery = this.#recoveryFor(record);
    const previousMetadata = record.metadata;
    const authorityAttributes = [
      "data-suprnova-live-snapshot",
      "data-suprnova-live-snapshot-kind",
      "data-suprnova-live-revision",
      "data-suprnova-live-instance",
    ] as const;
    const previousAttributes = new Map(
      authorityAttributes.map((name) => [name, record.element.getAttribute(name)]),
    );
    const models = this.#events.modelState(record);
    const previousValidation = new Map(
      models?.fields().map((field) => [field, models.snapshot(field).validation]) ?? [],
    );
    let commitApplied = false;
    const applicationActive = (): boolean => this.#current(record, connectionEpoch);
    const requestFreshRender = (): boolean => {
      let intent: ReturnType<typeof createFreshRenderIntent> | null = null;
      try {
        intent = createFreshRenderIntent(record);
        if (record.enqueue(intent)) return true;
        intent.finish("rejected");
      } catch {
        intent?.finish("rejected");
        // Recovery admission is closed and never falls back to replay or document reload.
      }
      return false;
    };
    return {
      applicationCurrent: (epoch) => applicationActive() && recovery.current(epoch),
      beginApplication: (response) =>
        recovery.begin({
          acceptedRevision: response.acceptedRevision,
          connectionEpoch,
        }),
      commit: (response) => {
        commitApplied = true;
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
      completeApplication: (epoch) => {
        if (!recovery.succeed(epoch)) return;
        record.scheduler.resetRecovery();
        this.#feedback.setRecovery(record, recovery.state());
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
      morph: async (prepared) => {
        const transitionLifecycle = this.#transitionsFor(record);
        let teleport: ReturnType<TeleportRegistry["begin"]> | null = null;
        try {
          continuity = captureContinuity(prepared.plan, {
            composition: this.#composition,
            signalScopes: this.#signals.capture(record),
          });
          featureMorphActive = true;
          this.#driveFeatureDriver(6, record.element);
          const transitions = prepareMorphTransitions(prepared.plan);
          await transitionLifecycle.begin(transitions.before).finished;
          if (!applicationActive()) throw new Error("island_application_retired");
          teleport = this.#teleports.begin(prepared.plan);
          const result = this.#morph.apply(prepared.plan, {});
          this.#teleports.commit(teleport, result.root);
          await transitionLifecycle.begin(transitions.after(result.root)).finished;
          if (!applicationActive()) throw new Error("island_application_retired");
          const metadata = parseIslandMetadata(result.root, this.#config);
          this.#assertSameMetadata(prepared.metadata, metadata);
          successorMetadata = metadata;
        } catch (error: unknown) {
          if (teleport?.active === true) this.#teleports.rollback(teleport);
          if (featureMorphActive) {
            featureMorphActive = false;
            this.#driveFeatureDriver(8, record.element);
          }
          continuity = null;
          throw error;
        }
      },
      navigate: (response) => {
        this.#transitions.get(record)?.cancel("navigation");
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
        if (models !== null) {
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
        }
        if (continuity !== null) {
          restoreContinuity(continuity, continuityRoot, {
            restoreSignals: (captured) => this.#signals.restore(record, captured),
          });
          if (featureMorphActive) {
            featureMorphActive = false;
            this.#driveFeatureDriver(7, continuityRoot);
          }
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
      recover: (error, response, epoch): RecoveryDecision => {
        void error;
        void response;
        if (featureMorphActive) {
          featureMorphActive = false;
          this.#driveFeatureDriver(8, record.element);
        }
        continuity = null;
        let decision = recovery.fail(epoch);
        if (
          decision.disposition === "request_fresh_render" &&
          (!record.scheduler.claimRecovery() || !requestFreshRender())
        ) {
          recovery.disconnect();
          decision = Object.freeze({ disposition: "disconnect_island" });
        }
        this.#feedback.setRecovery(record, recovery.state());
        this.#diagnostics.record({
          code: "response_invalid",
          detailCode: "recovery_required",
          phase: "response",
          severity: "error",
        });
        if (decision.disposition === "disconnect_island") this.#retire(record.element);
        return decision;
      },
      requestFreshIsland: () => {
        this.#ports.navigation.reload();
      },
      rollbackCommit: () => {
        if (!commitApplied || !applicationActive()) return;
        for (const [name, value] of previousAttributes) {
          if (value === null) record.element.removeAttribute(name);
          else record.element.setAttribute(name, value);
        }
        record.commitMetadata(previousMetadata);
        if (models !== null) {
          for (const [field, validation] of previousValidation) {
            models.setValidation(field, validation);
          }
        }
        commitApplied = false;
      },
      restoreFocus: () => {
        if (continuity === null) return;
        restoreContinuityFocus(continuity, continuityRoot);
        continuity = null;
      },
      retainDom: () => undefined,
      runEffects: (response) =>
        runValidatedEffects(response.effects, {
          effect: async (emission) => {
            await this.#effects.runAll(
              {
                active: applicationActive,
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

  #running(): boolean {
    return this.#state === "running";
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
        this.#teleports.disposeOwner(element);
        this.#retireFeatureDriver(record);
      });
      const directives = this.#ownership.connect(record);
      this.#signals.connect(record, directives);
      this.#events.connect(record, directives);
      this.#feedback.connect(record, directives, this.#events.modelState(record));
      this.#lazy.connect(record, directives);
      this.#teleports.mount(record.element);
      this.#connectFeatureDriver(record);
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

  #startFeatureDriver(): void {
    const driver = this.#featureDriver;
    if (driver === null || this.#state !== "running" || this.#featureDriverState !== 0) return;
    this.#featureDriverState = 1;
    const port: RuntimeFeatureDriverDocumentPort = Object.freeze({
      diagnose: (detail: RuntimeFeatureDiagnosticDetail) => {
        const candidate: unknown = detail;
        this.#featureDiagnostic(
          candidate === "contract_mismatch" ||
            candidate === "operation_rejected" ||
            candidate === "resource_exhausted"
            ? candidate
            : "operation_rejected",
        );
      },
      stimulus: this.#stimulus,
    });
    if (!this.#invokeFeatureDriver(driver, 0, port)) return;
    // The optional callback can synchronously dispose this runtime; TypeScript cannot model that reentrant mutation.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- re-read state after untyped callback reentry
    if (this.#state !== "running") return;
    this.#featureDriverState = 2;
    for (const record of this.#records.values()) this.#connectFeatureDriver(record);
  }

  #connectFeatureDriver(record: IslandRecord): void {
    const driver = this.#featureDriver;
    if (
      driver === null ||
      this.#featureDriverState !== 2 ||
      this.#state !== "running" ||
      this.#featureDriverClaims.has(record)
    ) {
      return;
    }
    this.#featureDriverClaims.add(record);
    const current = (): boolean => this.#state === "running" && record.active();
    const port: RuntimeFeatureDriverIslandPort = Object.freeze({
      authorizeRegisteredEvents: (registration: RegisteredBrowserEventRegistration) =>
        this.#registeredEvents.replace(record, registration, {
          current,
          event: (type, detail) => {
            const window = this.#document.defaultView;
            if (window === null) throw new Error("registered_event_document_retired");
            return new window.CustomEvent(type, {
              bubbles: false,
              composed: false,
              detail,
            });
          },
          targets: (target, maximumFanout) =>
            this.#registeredEventTargets(record, target, maximumFanout),
        }),
      dispatchRegisteredEvent: (
        capability: RegisteredBrowserEventCapability,
        event: RegisteredBrowserEventDispatch,
      ) => this.#dispatchRegisteredEvent(record, capability, event),
      element: record.element,
      enqueueFreshRender: (
        reason: FreshRenderReason,
        completion?: FreshRenderCompletionObserver,
      ) => {
        const candidate: unknown = reason;
        if (!current() || (candidate !== "poll" && candidate !== "stream")) {
          completion?.("retired");
          return "retired";
        }
        return completion === undefined
          ? record.enqueueFreshRender(candidate)
          : record.enqueueFreshRender(candidate, completion);
      },
      identity: Object.freeze({
        component: record.metadata.component,
        documentKey: record.metadata.documentKey,
        slot: record.metadata.slot,
      }),
      proposeUploadHandle: (field: string, proposal: UploadHandleProposal) =>
        this.#proposeUploadHandle(record, field, proposal, current),
      writePresentationSignal: (scope: string, name: string, value: JsonValue) => {
        if (!current()) {
          throw new Error("feature_signal_context_invalid");
        }
        return this.#signals.setDeclaredFromAsync(record, scope, name, value);
      },
    });
    this.#invokeFeatureDriver(driver, 1, port);
  }

  #dispatchRegisteredEvent(
    owner: IslandRecord,
    capability: RegisteredBrowserEventCapability,
    event: RegisteredBrowserEventDispatch,
  ): RegisteredBrowserEventDisposition {
    return this.#registeredEvents.dispatch(owner, capability, event);
  }

  #registeredEventTargets(
    record: IslandRecord,
    target: string,
    maximumFanout: number,
  ): readonly GuardedRegisteredEventTarget[] | "fanout_exceeded" {
    const targets: GuardedRegisteredEventTarget[] = [];
    const guard = (
      eventTarget: EventTarget,
      current: () => boolean,
    ): GuardedRegisteredEventTarget =>
      Object.freeze({
        current,
        dispatch: (event: Event) => eventTarget.dispatchEvent(event),
      });
    const currentIsland = (candidate: IslandRecord): boolean =>
      candidate.active() &&
      this.#records.get(candidate.element) === candidate &&
      this.#ownership.ownerForNode(candidate.element) === candidate;
    if (target === "self") {
      targets.push(guard(record.element, () => currentIsland(record)));
    } else if (target === "parent") {
      const parent = record.element.parentElement?.closest(ISLAND_ROOT_SELECTOR) ?? null;
      const parentRecord = parent === null ? undefined : this.#records.get(parent);
      if (parentRecord?.active() === true) {
        targets.push(
          guard(
            parentRecord.element,
            () =>
              currentIsland(parentRecord) &&
              record.element.parentElement?.closest(ISLAND_ROOT_SELECTOR) === parentRecord.element,
          ),
        );
      }
    } else if (target === "child") {
      for (const candidate of this.#records.values()) {
        if (!candidate.active() || candidate === record) continue;
        const parent = candidate.element.parentElement?.closest(ISLAND_ROOT_SELECTOR) ?? null;
        if (parent === record.element) {
          targets.push(
            guard(
              candidate.element,
              () =>
                currentIsland(candidate) &&
                candidate.element.parentElement?.closest(ISLAND_ROOT_SELECTOR) === record.element,
            ),
          );
        }
        if (targets.length > maximumFanout) return "fanout_exceeded";
      }
    } else if (target === "document") {
      targets.push(guard(this.#document, () => currentIsland(record)));
    } else if (target.startsWith("named_island:")) {
      const slot = target.slice("named_island:".length);
      for (const candidate of this.#records.values()) {
        if (candidate.active() && candidate.metadata.slot === slot) {
          targets.push(
            guard(
              candidate.element,
              () => currentIsland(candidate) && candidate.metadata.slot === slot,
            ),
          );
        }
        if (targets.length > maximumFanout) return "fanout_exceeded";
      }
    } else if (target.startsWith("browser:")) {
      const window = this.#document.defaultView;
      if (window !== null) {
        targets.push(
          guard(window, () => currentIsland(record) && this.#document.defaultView === window),
        );
      }
    }
    return targets;
  }

  #proposeUploadHandle(
    record: IslandRecord,
    field: string,
    proposal: UploadHandleProposal,
    current: () => boolean,
  ): UploadHandleProposalDisposition {
    return this.#uploadProposals.propose(record, field, proposal, {
      active: current,
      declared: (candidate) => declaresUploadField(record.element, candidate),
      write: (value) => this.#events.proposeTypedModel(record, field, value),
    });
  }

  #retireFeatureDriver(record: IslandRecord): void {
    this.#registeredEvents.retire(record);
    if (!this.#featureDriverClaims.delete(record)) return;
    this.#driveFeatureDriver(4, record.element);
  }

  #driveFeatureDriver(event: 2 | 3 | 4 | 6 | 7 | 8, value: Element | null): void {
    const driver = this.#featureDriver;
    if (driver !== null && this.#featureDriverState === 2) {
      this.#invokeFeatureDriver(driver, event, value);
    }
  }

  #invokeFeatureDriver(
    driver: InspectedRuntimeFeatureDriver,
    event: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8,
    value: RuntimeFeatureDriverValue,
  ): boolean {
    try {
      const completed: unknown = Reflect.apply(driver[4], driver, [event, value]);
      if (completed === true) return true;
    } catch {
      // The optional driver cannot expose thrown values or disable core lifecycle.
    }
    this.#featureDiagnostic("operation_rejected");
    return false;
  }

  #featureDiagnostic(detail: RuntimeFeatureDiagnosticDetail): void {
    this.#diagnostics.record({
      code: detail === "resource_exhausted" ? "resource_limit" : "lifecycle_notice",
      detailCode: detail,
      phase: "lifecycle",
      severity: "error",
    });
  }

  #recoveryFor(record: IslandRecord): ApplicationRecovery {
    let recovery = this.#recoveries.get(record);
    if (recovery !== undefined) return recovery;
    recovery = new ApplicationRecovery();
    this.#recoveries.set(record, recovery);
    const created = recovery;
    record.onDispose(() => {
      created.disconnect();
    });
    return recovery;
  }

  #transitionsFor(record: IslandRecord): TransitionLifecycle {
    let lifecycle = this.#transitions.get(record);
    if (lifecycle !== undefined) return lifecycle;
    lifecycle = new TransitionLifecycle(
      new TransitionRunner({
        completion: this.#transitionCompletion,
        prefersReducedMotion: () => this.#ports.features.prefersReducedMotion(),
        scheduler: this.#ports.scheduler,
      }),
    );
    this.#transitions.set(record, lifecycle);
    const created = lifecycle;
    record.onDispose(() => {
      created.dispose();
    });
    return lifecycle;
  }

  #mutations(mutations: readonly MutationRecord[]): void {
    if (this.#state !== "running") return;
    this.#navigation.mutations(mutations);
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
