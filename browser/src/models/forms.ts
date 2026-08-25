import type { JsonValue } from "../canonical.js";
import type { OwnedDirective } from "../directives/ownership.js";
import type { DirectiveOwnership } from "../directives/ownership.js";
import type { IslandRecord } from "../islands/record.js";
import type { RuntimeClock, RuntimeScheduler } from "../runtime/ports.js";
import type { ServerIntent, ServerOperation } from "../scheduler/intent.js";
import { FIFO_POLICY } from "../scheduler/policy.js";
import type { SchedulerPolicy } from "../scheduler/types.js";
import { controlEligibleForModel, readModelControl, type ModelControlRead } from "./control.js";
import { ModelState, type ModelEditResult } from "./state.js";
import {
  ModelTimingCoordinator,
  parseModelTiming,
  type ModelTimingEvent,
  type ModelTimingPolicy,
} from "./timing.js";
import { MISSING, immutableModelValue, isMissing, modelValuesEqual } from "./value.js";

const MODEL_DIRECTIVE_NAMES = new Set(["model"]);
const MAX_BINDINGS_PER_ISLAND = 512;

export interface ModelBatchSample {
  readonly field: string;
  readonly read: ModelControlRead;
  readonly editSequence: bigint;
  readonly eligible: boolean;
}

export interface ModelBatch {
  readonly operations: readonly ServerOperation[];
  readonly proposals: Readonly<Record<string, JsonValue>>;
  readonly editSequences: Readonly<Record<string, bigint>>;
}

interface SelectedSample {
  readonly value: JsonValue;
  readonly editSequence: bigint;
}

interface ModelBinding {
  readonly identity: string;
  readonly owned: OwnedDirective;
  readonly timing: ModelTimingPolicy;
  readonly policy: SchedulerPolicy;
}

export interface ModelDispatch {
  readonly batch: ModelBatch;
  readonly eventType: string;
  readonly owned: OwnedDirective;
  readonly policy: SchedulerPolicy;
  readonly trusted: boolean;
}

export type ModelDispatchSink = (dispatch: ModelDispatch) => ServerIntent | null;

export function buildModelBatch(samples: readonly ModelBatchSample[]): ModelBatch {
  const selected = new Map<string, SelectedSample>();
  for (const sample of samples) {
    if (
      !sample.eligible ||
      sample.read.kind === "missing" ||
      sample.read.kind === "unsupported_file"
    ) {
      continue;
    }
    if (sample.read.kind === "invalid") throw new Error("model_control_invalid");
    const value = immutableModelValue(sample.read.value);
    const existing = selected.get(sample.field);
    if (existing !== undefined && !modelValuesEqual(existing.value, value)) {
      throw new Error("model_control_ambiguous");
    }
    if (existing === undefined || sample.editSequence >= existing.editSequence) {
      selected.set(sample.field, { editSequence: sample.editSequence, value });
    }
  }

  const operations: ServerOperation[] = [];
  const proposals: Record<string, JsonValue> = {};
  const editSequences: Record<string, bigint> = {};
  for (const field of [...selected.keys()].sort()) {
    const sample = selected.get(field);
    if (sample === undefined) continue;
    operations.push(Object.freeze({ field, kind: "sync_model" }));
    proposals[field] = sample.value;
    editSequences[field] = sample.editSequence;
  }
  return Object.freeze({
    editSequences: Object.freeze(editSequences),
    operations: Object.freeze(operations),
    proposals: Object.freeze(proposals),
  });
}

export class ModelFormRuntime {
  readonly #ownership: DirectiveOwnership;
  readonly #clock: RuntimeClock;
  readonly #scheduler: RuntimeScheduler;
  readonly #sink: ModelDispatchSink;
  readonly #bindings = new WeakMap<object, ModelBinding>();
  readonly #byIsland = new Map<IslandRecord, Set<ModelBinding>>();
  readonly #states = new WeakMap<IslandRecord, ModelState>();
  readonly #typedFields = new Map<IslandRecord, Set<string>>();
  readonly #timings = new WeakMap<IslandRecord, ModelTimingCoordinator>();
  readonly #registeredRecords = new WeakSet<IslandRecord>();
  #bindingSequence = 0;
  #intentSequence = 0;

  constructor(
    ownership: DirectiveOwnership,
    clock: RuntimeClock,
    scheduler: RuntimeScheduler,
    sink: ModelDispatchSink,
  ) {
    this.#ownership = ownership;
    this.#clock = clock;
    this.#scheduler = scheduler;
    this.#sink = sink;
  }

  connect(record: IslandRecord, directives: readonly OwnedDirective[]): void {
    let bindings = this.#byIsland.get(record);
    if (bindings === undefined) {
      bindings = new Set();
      this.#byIsland.set(record, bindings);
    }
    if (!this.#registeredRecords.has(record)) {
      this.#registeredRecords.add(record);
      record.onDispose(() => {
        this.#retireRecord(record);
      });
    }
    const fields = new Set<string>();
    for (const owned of directives) {
      if (owned.directive.name !== "model" || this.#bindings.has(owned.directive)) continue;
      if (bindings.size >= MAX_BINDINGS_PER_ISLAND) throw new Error("model_binding_limit");
      const binding: ModelBinding = Object.freeze({
        identity: this.#nextBindingIdentity(record, owned.directive.value),
        owned,
        policy: schedulerPolicy(owned),
        timing: parseModelTiming(owned.directive.modifiers),
      });
      bindings.add(binding);
      this.#bindings.set(owned.directive, binding);
      fields.add(owned.directive.value);
    }
    const state = this.#state(record);
    for (const field of fields) {
      const read = this.#readField(record, field);
      state.register(field, readToValue(read));
    }
  }

  route(event: Event, phase: "capture" | "bubble"): void {
    if (event.type === "blur" && phase !== "capture") return;
    if (event.type !== "blur" && phase !== "bubble") return;
    if (event.type === "reset") {
      const form = asForm(event.composedPath()[0] ?? event.target);
      if (form === null) return;
      const trusted = event.isTrusted;
      this.#scheduler.microtask(() => {
        this.#resetForm(form, trusted);
      });
      return;
    }
    if (event.type !== "input" && event.type !== "change" && event.type !== "blur") return;
    const owned = this.#ownership.resolveNamed(event.composedPath(), MODEL_DIRECTIVE_NAMES);
    if (owned === null || !owned.island.active() || !owned.element.isConnected) return;
    const binding = this.#bindings.get(owned.directive);
    if (binding === undefined || !controlEligibleForModel(owned.element)) return;
    this.#capture(binding, event.type, event.isTrusted);
  }

  prepareAction(owned: OwnedDirective, eventType: string): ModelBatch {
    if (eventType === "init") return emptyBatch();
    const bindings = this.#byIsland.get(owned.island);
    let selected: ModelBinding[];
    if (bindings === undefined) {
      selected = [];
    } else if (eventType === "submit") {
      const form = asForm(owned.element);
      if (form === null) return emptyBatch();
      selected = [...bindings].filter((binding) => associatedWithForm(binding.owned.element, form));
    } else {
      selected = [...bindings].filter(
        (binding) => binding.timing.kind === "action" || binding.owned.element === owned.element,
      );
    }
    return mergeModelBatches(
      this.#sampleBatch(owned.island, selected, true),
      this.#typedBatch(owned.island),
    );
  }

  proposeTyped(record: IslandRecord, field: string, value: JsonValue): ModelEditResult {
    let fields = this.#typedFields.get(record);
    if (fields === undefined) {
      fields = new Set();
      this.#typedFields.set(record, fields);
    }
    if (!fields.has(field)) {
      if (fields.size >= MAX_BINDINGS_PER_ISLAND) throw new Error("model_binding_limit");
      fields.add(field);
      this.#state(record).register(field);
    }
    return this.#state(record).propose(field, value);
  }

  trackIntent(record: IslandRecord, batch: ModelBatch, intent: ServerIntent): void {
    const fields = Object.keys(batch.proposals);
    if (fields.length === 0) return;
    this.#intentSequence = incrementSequence(
      this.#intentSequence,
      "model_intent_sequence_exhausted",
    );
    const identity = `intent-${String(this.#intentSequence)}`;
    const state = this.#state(record);
    for (const field of fields) state.markInFlight(field, identity);
    intent.onFinish(() => {
      for (const field of fields) state.clearInFlight(field, identity);
    });
  }

  state(record: IslandRecord): ModelState | null {
    return this.#states.get(record) ?? null;
  }

  suspend(): void {
    for (const record of this.#byIsland.keys()) this.#timings.get(record)?.cancelAll();
  }

  retireSubtree(record: IslandRecord, node: Node): void {
    const bindings = this.#byIsland.get(record);
    if (bindings === undefined) return;
    for (const binding of [...bindings]) {
      if (node === binding.owned.element || node.contains(binding.owned.element)) {
        this.#retireBinding(bindings, binding);
      }
    }
  }

  #capture(binding: ModelBinding, event: ModelTimingEvent, trusted: boolean): void {
    const record = binding.owned.island;
    const field = binding.owned.directive.value;
    const read = this.#readField(record, field);
    if (read.kind === "invalid" || read.kind === "unsupported_file") return;
    const state = this.#state(record);
    const edit = state.propose(field, readToValue(read));
    const boundary =
      (binding.timing.kind === "change" && event === "change") ||
      (binding.timing.kind === "blur" && event === "blur");
    if (!edit.changed && !boundary) return;
    this.#timing(record).update(binding.identity, binding.timing, event, () => {
      this.#dispatchBinding(binding, event, trusted);
    });
  }

  #dispatchBinding(binding: ModelBinding, eventType: string, trusted: boolean): void {
    const record = binding.owned.island;
    if (!record.active() || !binding.owned.element.isConnected) return;
    const state = this.#state(record);
    const proposal = state.proposal(binding.owned.directive.value);
    if (isMissing(proposal)) return;
    const batch = buildModelBatch([
      {
        editSequence: state.editSequence(binding.owned.directive.value),
        eligible: controlEligibleForModel(binding.owned.element),
        field: binding.owned.directive.value,
        read: { kind: "value", value: proposal },
      },
    ]);
    const intent = this.#sink(
      Object.freeze({ batch, eventType, owned: binding.owned, policy: binding.policy, trusted }),
    );
    if (intent !== null) this.trackIntent(record, batch, intent);
  }

  #sampleBatch(
    record: IslandRecord,
    bindings: readonly ModelBinding[],
    cancelTiming: boolean,
  ): ModelBatch {
    const grouped = groupBindings(bindings);
    const state = this.#state(record);
    const samples: ModelBatchSample[] = [];
    for (const [field, group] of grouped) {
      if (cancelTiming) {
        for (const binding of group) this.#timing(record).cancel(binding.identity);
      }
      const read = readBindingGroup(group);
      if (read.kind === "value") state.propose(field, read.value);
      else if (read.kind === "missing") state.propose(field, MISSING);
      samples.push({
        editSequence: state.editSequence(field),
        eligible: group.some((binding) => controlEligibleForModel(binding.owned.element)),
        field,
        read,
      });
    }
    return buildModelBatch(samples);
  }

  #typedBatch(record: IslandRecord): ModelBatch {
    const state = this.#state(record);
    const samples: ModelBatchSample[] = [];
    for (const field of this.#typedFields.get(record) ?? []) {
      const proposal = state.proposal(field);
      if (isMissing(proposal)) continue;
      samples.push({
        editSequence: state.editSequence(field),
        eligible: true,
        field,
        read: { kind: "value", value: proposal },
      });
    }
    return buildModelBatch(samples);
  }

  #readField(record: IslandRecord, field: string): ModelControlRead {
    const bindings = [...(this.#byIsland.get(record) ?? [])].filter(
      (binding) => binding.owned.directive.value === field,
    );
    return readBindingGroup(bindings);
  }

  #resetForm(form: HTMLFormElement, trusted: boolean): void {
    for (const [record, bindings] of this.#byIsland) {
      if (!record.active()) continue;
      for (const binding of bindings) {
        if (!associatedWithForm(binding.owned.element, form)) continue;
        this.#capture(binding, "reset", trusted);
      }
    }
  }

  #state(record: IslandRecord): ModelState {
    let state = this.#states.get(record);
    if (state === undefined) {
      state = new ModelState();
      this.#states.set(record, state);
    }
    return state;
  }

  #timing(record: IslandRecord): ModelTimingCoordinator {
    let timing = this.#timings.get(record);
    if (timing === undefined) {
      timing = new ModelTimingCoordinator(this.#clock, this.#scheduler, MAX_BINDINGS_PER_ISLAND);
      this.#timings.set(record, timing);
    }
    return timing;
  }

  #nextBindingIdentity(record: IslandRecord, field: string): string {
    this.#bindingSequence = incrementSequence(
      this.#bindingSequence,
      "model_binding_sequence_exhausted",
    );
    return `${record.metadata.documentKey}:${field}:${String(this.#bindingSequence)}`;
  }

  #retireBinding(bindings: Set<ModelBinding>, binding: ModelBinding): void {
    this.#timings.get(binding.owned.island)?.cancel(binding.identity);
    bindings.delete(binding);
    this.#bindings.delete(binding.owned.directive);
  }

  #retireRecord(record: IslandRecord): void {
    const bindings = this.#byIsland.get(record);
    if (bindings === undefined) return;
    for (const binding of [...bindings]) this.#retireBinding(bindings, binding);
    this.#timings.get(record)?.dispose();
    this.#typedFields.delete(record);
    this.#byIsland.delete(record);
  }
}

function schedulerPolicy(owned: OwnedDirective): SchedulerPolicy {
  const modifiers = owned.directive.modifiers.filter((modifier) =>
    ["latest", "parallel", "serial"].includes(modifier),
  );
  if (modifiers.length > 1) throw new Error("model_scheduler_policy_conflict");
  const key = `model:${owned.directive.value}`;
  switch (modifiers[0] ?? "latest") {
    case "latest":
      return Object.freeze({ abortInFlight: false, key, kind: "latest_only" });
    case "parallel":
      return Object.freeze({
        group: key,
        kind: "parallel",
        maximum: owned.island.parallelCapacity,
      });
    case "serial":
      return FIFO_POLICY;
    default:
      throw new Error("model_scheduler_policy_invalid");
  }
}

function groupBindings(bindings: readonly ModelBinding[]): Map<string, ModelBinding[]> {
  const grouped = new Map<string, ModelBinding[]>();
  for (const binding of bindings) {
    const field = binding.owned.directive.value;
    const group = grouped.get(field) ?? [];
    group.push(binding);
    grouped.set(field, group);
  }
  return grouped;
}

function readBindingGroup(bindings: readonly ModelBinding[]): ModelControlRead {
  const eligible = bindings.filter((binding) => controlEligibleForModel(binding.owned.element));
  if (eligible.length === 0) return Object.freeze({ kind: "missing" });
  const radios = eligible.filter((binding) => {
    const element = binding.owned.element;
    return (
      element.tagName.toUpperCase() === "INPUT" &&
      (element as HTMLInputElement).type.toLowerCase() === "radio"
    );
  });
  if (radios.length > 0) {
    if (radios.length !== eligible.length) {
      return Object.freeze({ code: "control_unsupported", kind: "invalid" });
    }
    const selected = radios.find((binding) => (binding.owned.element as HTMLInputElement).checked);
    return selected === undefined
      ? Object.freeze({ kind: "missing" })
      : readModelControl(selected.owned.element);
  }
  const reads = eligible.map((binding) => readModelControl(binding.owned.element));
  const invalid = reads.find((read) => read.kind === "invalid");
  if (invalid !== undefined) return invalid;
  const file = reads.find((read) => read.kind === "unsupported_file");
  if (file !== undefined)
    return reads.length === 1
      ? file
      : Object.freeze({ code: "control_unsupported", kind: "invalid" });
  const values = reads.filter(
    (read): read is Readonly<{ kind: "value"; value: JsonValue }> => read.kind === "value",
  );
  if (values.length === 0) return Object.freeze({ kind: "missing" });
  if (values.some((candidate) => !modelValuesEqual(candidate.value, values[0]?.value ?? MISSING))) {
    return Object.freeze({ code: "control_unsupported", kind: "invalid" });
  }
  return values[0] ?? Object.freeze({ kind: "missing" });
}

function readToValue(read: ModelControlRead): JsonValue | typeof MISSING {
  return read.kind === "value" ? read.value : MISSING;
}

function asForm(target: EventTarget | null): HTMLFormElement | null {
  if (!(target instanceof Element) || target.tagName.toUpperCase() !== "FORM") return null;
  return target as HTMLFormElement;
}

function associatedWithForm(element: Element, form: HTMLFormElement): boolean {
  if ("form" in element && element.form !== undefined) return element.form === form;
  return form.contains(element);
}

function mergeModelBatches(left: ModelBatch, right: ModelBatch): ModelBatch {
  if (right.operations.length === 0) return left;
  if (left.operations.length === 0) return right;
  const duplicate = Object.keys(right.proposals).find((field) => field in left.proposals);
  if (duplicate !== undefined) throw new Error("model_control_ambiguous");
  return Object.freeze({
    editSequences: Object.freeze({ ...left.editSequences, ...right.editSequences }),
    operations: Object.freeze([...left.operations, ...right.operations]),
    proposals: Object.freeze({ ...left.proposals, ...right.proposals }),
  });
}

function emptyBatch(): ModelBatch {
  return Object.freeze({
    editSequences: Object.freeze({}),
    operations: Object.freeze([]),
    proposals: Object.freeze({}),
  });
}

function incrementSequence(value: number, code: string): number {
  if (value >= Number.MAX_SAFE_INTEGER) throw new Error(code);
  return value + 1;
}
