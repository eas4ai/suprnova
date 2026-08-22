import { DIRECTIVE_EVENT_TYPES } from "../generated/directive-contract.js";
import type { IslandRecord } from "../islands/record.js";
import { ModelFormRuntime, type ModelDispatch } from "../models/forms.js";
import type { ModelState } from "../models/state.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import { DelegatedListenerRegistry } from "../runtime/listeners.js";
import type { RuntimeClock, RuntimeRandomness, RuntimeScheduler } from "../runtime/ports.js";
import type { JsonValue } from "../canonical.js";
import {
  createServerIntent,
  type IntentSource,
  type ServerIntent,
  type ServerOperation,
} from "../scheduler/intent.js";
import {
  applyEventEffects,
  evaluateEventModifiers,
  type DelegatedEventPhase,
} from "./modifiers.js";
import { DirectiveOwnership, type OwnedDirective } from "./ownership.js";

const EVENT_TYPES: readonly string[] = DIRECTIVE_EVENT_TYPES;
const ROUTED_EVENT_TYPES = Object.freeze([...new Set([...EVENT_TYPES, "blur", "reset"])]);
const MAX_LOCAL_EVENT_HANDLERS = 16;

export type LocalEventHandler = (event: Event) => void;

export class EventRouter {
  readonly #ownership: DirectiveOwnership;
  readonly #randomness: RuntimeRandomness;
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #models: ModelFormRuntime;
  readonly #once = new WeakSet();
  readonly #localHandlers = new Map<string, Set<LocalEventHandler>>();

  constructor(
    listeners: DelegatedListenerRegistry,
    ownership: DirectiveOwnership,
    randomness: RuntimeRandomness,
    clock: RuntimeClock,
    scheduler: RuntimeScheduler,
    diagnostics: RuntimeDiagnosticSink,
  ) {
    this.#ownership = ownership;
    this.#randomness = randomness;
    this.#diagnostics = diagnostics;
    this.#models = new ModelFormRuntime(ownership, clock, scheduler, (dispatch) =>
      this.#scheduleModel(dispatch),
    );
    for (const eventType of ROUTED_EVENT_TYPES) {
      listeners.add(
        eventType,
        (event) => {
          this.#dispatch(event, "capture");
        },
        { capture: true },
      );
      listeners.add(eventType, (event) => {
        this.#dispatch(event, "bubble");
      });
    }
  }

  onLocal(eventType: string, handler: LocalEventHandler): VoidFunction {
    if (!EVENT_TYPES.some((candidate) => candidate === eventType)) {
      throw new Error("local_event_type_rejected");
    }
    let handlers = this.#localHandlers.get(eventType);
    if (handlers === undefined) {
      handlers = new Set();
      this.#localHandlers.set(eventType, handlers);
    }
    if (handlers.size >= MAX_LOCAL_EVENT_HANDLERS || handlers.has(handler)) {
      throw new Error("local_event_handler_rejected");
    }
    handlers.add(handler);
    let removed = false;
    return () => {
      if (removed) return;
      removed = true;
      handlers.delete(handler);
    };
  }

  connect(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    this.#models.connect(record, directives);
    this.#scheduleInitial(record, directives);
  }

  modelState(record: IslandRecord): ModelState | null {
    return this.#models.state(record);
  }

  suspend(): void {
    this.#models.suspend();
  }

  schedulePublicCall(owned: OwnedDirective, name: string, input: JsonValue): boolean {
    if (
      owned.directive.name !== "call" ||
      owned.directive.value !== name ||
      input === null ||
      typeof input !== "object" ||
      Array.isArray(input)
    ) {
      return false;
    }
    const source: IntentSource = Object.freeze({
      island: owned.island,
      element: owned.element,
      directive: owned.directive,
      eventType: "call",
      trusted: false,
    });
    try {
      const batch = this.#models.prepareAction(owned, "call");
      const operations: ServerOperation[] = [
        ...batch.operations,
        {
          kind: "invoke_action",
          name,
          arguments: input as Readonly<Record<string, JsonValue>>,
        },
      ];
      const intent = createServerIntent(
        source,
        operations,
        this.#randomness,
        owned.island.metadata.snapshotForm === "seed",
        batch.proposals,
        batch.editSequences,
      );
      if (owned.island.enqueue(intent)) {
        this.#models.trackIntent(owned.island, batch, intent);
        return true;
      }
      intent.finish("rejected");
    } catch {
      // The caller receives one closed scheduling failure.
    }
    this.#diagnostics.record({
      code: "scheduler_rejected",
      severity: "error",
      phase: "schedule",
      detailCode: "operation_rejected",
    });
    return false;
  }

  scanInsertion(record: IslandRecord, node: Node, trusted: boolean): readonly OwnedDirective[] {
    const directives = this.#ownership.scanInsertion(record, node, trusted);
    this.#models.connect(record, directives);
    this.#scheduleInitial(record, directives);
    return directives;
  }

  retireSubtree(record: IslandRecord, node: Node): void {
    this.#models.retireSubtree(record, node);
    this.#ownership.retireSubtree(record, node);
  }

  #scheduleInitial(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    for (const owned of directives) {
      if (owned.directive.name !== "init" || !record.active()) continue;
      this.#schedule(owned, "init", true);
    }
  }

  #route(event: Event, phase: DelegatedEventPhase): void {
    const path = event.composedPath();
    const owned = this.#ownership.resolve(path, event.type, phase);
    if (owned === null || !owned.island.active() || !owned.element.isConnected) return;
    if (this.#once.has(owned.directive)) return;
    const origin = path[0] ?? event.target;
    const decision = evaluateEventModifiers(owned.directive, event, owned.element, origin, phase);
    if (decision === null || !this.#schedule(owned, event.type, event.isTrusted)) return;
    applyEventEffects(event, decision);
    if (decision.once) this.#once.add(owned.directive);
  }

  #dispatch(event: Event, phase: DelegatedEventPhase): void {
    this.#route(event, phase);
    try {
      this.#models.route(event, phase);
    } catch {
      this.#diagnostics.record({
        code: "directive_invalid",
        severity: "error",
        phase: "directive",
        detailCode: "operation_rejected",
      });
    }
    if (phase !== "bubble") return;
    for (const handler of this.#localHandlers.get(event.type) ?? []) {
      try {
        handler(event);
      } catch {
        this.#diagnostics.record({
          code: "directive_invalid",
          severity: "error",
          phase: "directive",
          detailCode: "operation_rejected",
        });
      }
    }
  }

  #schedule(owned: OwnedDirective, eventType: string, trusted: boolean): boolean {
    const source: IntentSource = Object.freeze({
      island: owned.island,
      element: owned.element,
      directive: owned.directive,
      eventType,
      trusted,
    });
    try {
      const batch = this.#models.prepareAction(owned, eventType);
      const operations: ServerOperation[] = [
        ...batch.operations,
        {
          kind: "invoke_action",
          name: owned.directive.value,
          arguments: Object.freeze({}),
        },
      ];
      const intent = createServerIntent(
        source,
        operations,
        this.#randomness,
        owned.island.metadata.snapshotForm === "seed",
        batch.proposals,
        batch.editSequences,
      );
      if (owned.island.enqueue(intent)) {
        this.#models.trackIntent(owned.island, batch, intent);
        return true;
      }
      intent.finish("rejected");
    } catch {
      this.#diagnostics.record({
        code: "scheduler_rejected",
        severity: "error",
        phase: "schedule",
        detailCode: "operation_rejected",
      });
    }
    return false;
  }

  #scheduleModel(dispatch: ModelDispatch): ServerIntent | null {
    const source: IntentSource = Object.freeze({
      directive: dispatch.owned.directive,
      element: dispatch.owned.element,
      eventType: dispatch.eventType,
      island: dispatch.owned.island,
      trusted: dispatch.trusted,
    });
    let intent: ServerIntent | null = null;
    try {
      intent = createServerIntent(
        source,
        dispatch.batch.operations,
        this.#randomness,
        dispatch.owned.island.metadata.snapshotForm === "seed",
        dispatch.batch.proposals,
        dispatch.batch.editSequences,
      );
      if (dispatch.owned.island.enqueue(intent, dispatch.policy)) return intent;
      intent.finish("rejected");
    } catch {
      intent?.finish("rejected");
    }
    this.#diagnostics.record({
      code: "scheduler_rejected",
      severity: "error",
      phase: "schedule",
      detailCode: "operation_rejected",
    });
    return null;
  }
}
