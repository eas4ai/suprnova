import { DirectiveOwnership, type OwnedDirective } from "../directives/ownership.js";
import type { EventRouter } from "../directives/events.js";
import type { IslandRecord } from "../islands/record.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import type { RuntimeScheduler } from "../runtime/ports.js";
import { SignalGraph } from "./graph.js";
import { buildPresentationBindings } from "./presentation.js";
import { LocalSignalScope } from "./scope.js";
import { parseSignalDeclarations, type SignalValue } from "./value.js";
import type { JsonValue } from "../canonical.js";

const TOGGLE_DIRECTIVES = new Set(["toggle"]);
const PRESENTATION_DIRECTIVES = new Set([
  "attr",
  "class",
  "expanded",
  "focus",
  "inert",
  "selected",
  "show",
]);
const SAFE_SCOPE_KEY = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
const MAX_SCOPES_PER_ISLAND = 256;

export interface SignalContinuity {
  readonly identity: string;
  readonly values: Readonly<Record<string, SignalValue>>;
}

interface RecordSignals {
  readonly scopes: Map<Element, LocalSignalScope>;
  readonly identities: Set<string>;
  readonly disposers: Map<Element, VoidFunction[]>;
}

function parentAcrossShadow(element: Element): Element | null {
  if (element.parentElement !== null) return element.parentElement;
  const root = element.getRootNode();
  return root instanceof ShadowRoot ? root.host : null;
}

function containedBy(root: Node, candidate: Element): boolean {
  let current: Element | null = candidate;
  while (current !== null) {
    if (current === root) return true;
    current = parentAcrossShadow(current);
  }
  return false;
}

export class SignalRuntime {
  readonly #ownership: DirectiveOwnership;
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #graph: SignalGraph;
  readonly #records = new Map<IslandRecord, RecordSignals>();
  readonly #scopeByElement = new WeakMap<Element, LocalSignalScope>();
  readonly #removeClickHandler: VoidFunction;

  constructor(
    events: EventRouter,
    ownership: DirectiveOwnership,
    scheduler: RuntimeScheduler,
    diagnostics: RuntimeDiagnosticSink,
  ) {
    this.#ownership = ownership;
    this.#diagnostics = diagnostics;
    this.#graph = new SignalGraph(scheduler);
    this.#removeClickHandler = events.onLocal("click", (event) => {
      this.#toggle(event);
    });
  }

  connect(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    if (this.#records.has(record)) return;
    this.#records.set(record, {
      scopes: new Map(),
      identities: new Set(),
      disposers: new Map(),
    });
    record.onDispose(() => {
      this.#retireRecord(record);
    });
    this.#process(record, directives);
  }

  scanInsertion(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    this.#process(record, directives);
  }

  retireSubtree(record: IslandRecord, node: Node): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    for (const [element, disposers] of [...state.disposers]) {
      if (!containedBy(node, element)) continue;
      for (const dispose of disposers) dispose();
      state.disposers.delete(element);
    }
    for (const [element, scope] of [...state.scopes]) {
      if (!containedBy(node, element)) continue;
      state.identities.delete(scope.identity);
      state.scopes.delete(element);
      this.#scopeByElement.delete(element);
      scope.dispose();
    }
  }

  capture(record: IslandRecord): readonly SignalContinuity[] {
    const state = this.#records.get(record);
    if (state === undefined) return [];
    return Object.freeze(
      [...state.scopes.values()].map((scope) =>
        Object.freeze({ identity: scope.identity, values: scope.values() }),
      ),
    );
  }

  restore(record: IslandRecord, continuity: readonly SignalContinuity[]): number {
    const state = this.#records.get(record);
    if (state === undefined) return 0;
    const scopes = new Map([...state.scopes.values()].map((scope) => [scope.identity, scope]));
    const seen = new Set<string>();
    let restored = 0;
    for (const captured of continuity) {
      if (seen.has(captured.identity)) {
        this.#rejectDirective();
        continue;
      }
      seen.add(captured.identity);
      const scope = scopes.get(captured.identity);
      if (scope?.restore(captured.values) === true) {
        this.#graph.changed(scope, Object.keys(captured.values));
        restored += 1;
      }
    }
    return restored;
  }

  setFromCall(record: IslandRecord, element: Element, name: string, input: JsonValue): JsonValue {
    const scope = this.#nearestScope(record, element);
    if (
      scope === null ||
      !(
        input === null ||
        typeof input === "boolean" ||
        typeof input === "string" ||
        (typeof input === "number" && Number.isSafeInteger(input))
      )
    ) {
      throw new Error("signal_call_invalid");
    }
    scope.set(name, input);
    return scope.get(name);
  }

  setDeclaredFromAsync(
    record: IslandRecord,
    scopeIdentity: string,
    name: string,
    input: JsonValue,
  ): JsonValue {
    const state = this.#records.get(record);
    if (state === undefined || !SAFE_SCOPE_KEY.test(scopeIdentity)) {
      throw new Error("signal_async_scope_invalid");
    }
    const entry = [...state.scopes].find(([, scope]) => scope.identity === scopeIdentity);
    if (
      entry === undefined ||
      !record.active() ||
      this.#ownership.ownerForNode(entry[0]) !== record ||
      !containedBy(record.element, entry[0]) ||
      this.#scopeByElement.get(entry[0]) !== entry[1] ||
      !(
        input === null ||
        typeof input === "boolean" ||
        typeof input === "string" ||
        (typeof input === "number" && Number.isSafeInteger(input))
      )
    ) {
      throw new Error("signal_async_scope_invalid");
    }
    entry[1].setDeclared(name, input);
    return entry[1].get(name);
  }

  dispose(): void {
    this.#removeClickHandler();
    for (const record of [...this.#records.keys()]) this.#retireRecord(record);
    this.#graph.dispose();
  }

  #process(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    for (const owned of directives) {
      if (owned.directive.name === "signal") this.#createScope(record, state, owned);
    }
    const pending: {
      readonly owned: OwnedDirective;
      readonly scope: LocalSignalScope;
      readonly binding: ReturnType<typeof buildPresentationBindings>[number];
    }[] = [];
    for (const owned of directives) {
      if (!PRESENTATION_DIRECTIVES.has(owned.directive.name)) continue;
      const scope = this.#nearestScope(record, owned.element);
      if (scope === null) {
        this.#rejectDirective();
        continue;
      }
      for (const binding of buildPresentationBindings(owned, scope, this.#diagnostics)) {
        pending.push({ owned, scope, binding });
      }
    }
    for (const { owned, scope, binding } of pending) {
      try {
        const dispose = this.#graph.register(scope, binding.signal, binding.target);
        const elementDisposers = state.disposers.get(owned.element) ?? [];
        elementDisposers.push(dispose);
        state.disposers.set(owned.element, elementDisposers);
      } catch {
        binding.target.dispose();
        this.#rejectDirective();
      }
    }
  }

  #createScope(record: IslandRecord, state: RecordSignals, owned: OwnedDirective): void {
    if (state.scopes.size >= MAX_SCOPES_PER_ISLAND || state.scopes.has(owned.element)) {
      this.#rejectDirective();
      return;
    }
    const identity =
      owned.element === record.element
        ? record.metadata.documentKey
        : owned.element.getAttribute("data-suprnova-live-key");
    if (identity === null || !SAFE_SCOPE_KEY.test(identity) || state.identities.has(identity)) {
      this.#rejectDirective();
      return;
    }
    try {
      const parent = this.#nearestScope(record, parentAcrossShadow(owned.element));
      const scope = new LocalSignalScope(
        identity,
        parseSignalDeclarations(owned.directive.value),
        parent,
        (changedScope, names) => {
          this.#graph.changed(changedScope, names);
        },
      );
      state.scopes.set(owned.element, scope);
      state.identities.add(identity);
      this.#scopeByElement.set(owned.element, scope);
    } catch {
      this.#rejectDirective();
    }
  }

  #nearestScope(record: IslandRecord, start: Element | null): LocalSignalScope | null {
    let element = start;
    while (element !== null) {
      const scope = this.#scopeByElement.get(element);
      if (scope !== undefined) return scope;
      if (element === record.element) return null;
      element = parentAcrossShadow(element);
    }
    return null;
  }

  #toggle(event: Event): void {
    const owned = this.#ownership.resolveNamed(event.composedPath(), TOGGLE_DIRECTIVES);
    if (
      owned === null ||
      !owned.island.active() ||
      !owned.element.isConnected ||
      owned.element.hasAttribute("disabled") ||
      owned.element.getAttribute("aria-disabled") === "true"
    ) {
      return;
    }
    const scope = this.#nearestScope(owned.island, owned.element);
    if (scope === null) {
      this.#rejectDirective();
      return;
    }
    try {
      scope.toggle(owned.directive.value);
    } catch {
      this.#rejectDirective();
    }
  }

  #retireRecord(record: IslandRecord): void {
    const state = this.#records.get(record);
    if (state === undefined) return;
    for (const disposers of state.disposers.values()) {
      for (const dispose of disposers) dispose();
    }
    for (const [element, scope] of state.scopes) {
      this.#scopeByElement.delete(element);
      scope.dispose();
    }
    this.#records.delete(record);
  }

  #rejectDirective(): void {
    this.#diagnostics.record({
      code: "directive_invalid",
      severity: "error",
      phase: "directive",
      detailCode: "operation_rejected",
    });
  }
}
