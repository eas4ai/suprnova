import { DIRECTIVE_CONTRACTS } from "../generated/directive-contract.js";
import type { IslandRecord } from "../islands/record.js";
import type { RuntimeDiagnostics } from "../runtime/diagnostics.js";
import { DelegatedListenerRegistry } from "../runtime/listeners.js";
import type { RuntimeRandomness } from "../runtime/ports.js";
import { createServerIntent, type IntentSource } from "../scheduler/intent.js";
import {
  applyEventEffects,
  evaluateEventModifiers,
  type DelegatedEventPhase,
} from "./modifiers.js";
import { DirectiveOwnership, type OwnedDirective } from "./ownership.js";

const EVENT_TYPES = Object.freeze(
  DIRECTIVE_CONTRACTS.filter(
    (contract) =>
      contract.phase === "schedule" && contract.value === "action" && contract.name !== "init",
  ).map((contract) => contract.name),
);
const MAX_LOCAL_EVENT_HANDLERS = 16;

export type LocalEventHandler = (event: Event) => void;

export class EventRouter {
  readonly #ownership: DirectiveOwnership;
  readonly #randomness: RuntimeRandomness;
  readonly #diagnostics: RuntimeDiagnostics;
  readonly #once = new WeakSet();
  readonly #localHandlers = new Map<string, Set<LocalEventHandler>>();

  constructor(
    listeners: DelegatedListenerRegistry,
    ownership: DirectiveOwnership,
    randomness: RuntimeRandomness,
    diagnostics: RuntimeDiagnostics,
  ) {
    this.#ownership = ownership;
    this.#randomness = randomness;
    this.#diagnostics = diagnostics;
    for (const eventType of EVENT_TYPES) {
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
    this.#scheduleInitial(record, directives);
  }

  scanInsertion(record: IslandRecord, node: Node): readonly OwnedDirective[] {
    const directives = this.#ownership.scanInsertion(record, node);
    this.#scheduleInitial(record, directives);
    return directives;
  }

  retireSubtree(record: IslandRecord, node: Node): void {
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
      const intent = createServerIntent(
        source,
        [{ kind: "invoke_action", name: owned.directive.value, arguments: Object.freeze({}) }],
        this.#randomness,
        owned.island.metadata.snapshotForm === "seed",
      );
      if (owned.island.enqueue(intent)) return true;
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
}
