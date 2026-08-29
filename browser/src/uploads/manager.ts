import {
  BoundedOwner,
  type BoundedDisposable,
  type BoundedLease,
  type PermitRequest,
} from "../features/bounded.js";
import {
  detachUploadCancellation,
  settleUploadCancellation,
  UploadTransfer,
  type UploadCancellationCleanup,
} from "./transfer.js";
import { reacquireUpload } from "./resume.js";
import {
  MAX_UPLOAD_ACTIVE_TRANSFERS,
  MAX_UPLOAD_CHUNK_BYTES,
  MAX_UPLOAD_FILES_PER_DOCUMENT,
  MAX_UPLOAD_QUEUE_BYTES,
  validateUploadField,
  type ReacquiredTransfer,
  type UploadHandle,
  type UploadIslandPort,
  type UploadManagerOptions,
  type UploadManagerResourceSnapshot,
  type UploadManagerSnapshot,
  type UploadSecretSnapshot,
  type UploadSelection,
} from "./types.js";

interface Binding {
  readonly field: string;
  readonly input: HTMLInputElement;
  readonly island: UploadIslandPort;
  readonly multiple: boolean;
  readonly transfers: Entry[];
}

interface Entry {
  readonly binding: Binding;
  readonly resource: BoundedDisposable;
  readonly transfer: UploadTransfer;
  handle: UploadHandle | null;
  lease: BoundedLease | null;
  permit: PermitRequest | null;
  settle: (() => void) | null;
  settled: Promise<void> | null;
  work: Promise<void> | null;
}

type UploadManagerObserver = (snapshot: UploadManagerSnapshot) => void;

class UploadCleanupOwner {
  readonly #active = new Set<object>();
  readonly #maxItems: number;
  #retired = false;

  constructor(maxItems: number) {
    this.#maxItems = maxItems;
  }

  schedule(cleanup: UploadCancellationCleanup): void {
    if (this.#retired || this.#active.size >= this.#maxItems) {
      detachUploadCancellation(cleanup);
      return;
    }
    const obligation = Object.freeze({});
    this.#active.add(obligation);
    void settleUploadCancellation(cleanup).finally(() => {
      this.#active.delete(obligation);
    });
  }

  retire(): void {
    this.#retired = true;
  }

  size(): number {
    return this.#active.size;
  }
}

function validLimit(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 1;
}

function sameIsland(left: UploadIslandPort, right: UploadIslandPort): boolean {
  return left === right || left.element === right.element;
}

export class UploadManager {
  readonly #options: UploadManagerOptions;
  readonly #owner: BoundedOwner<UploadTransfer>;
  readonly #entries = new Map<UploadTransfer, Entry>();
  readonly #bindings = new Map<UploadIslandPort, Map<string, Binding>>();
  readonly #cleanupOwner: UploadCleanupOwner;
  readonly #generations = new Map<UploadIslandPort, Map<string, object>>();
  readonly #observers = new Map<UploadIslandPort, Set<UploadManagerObserver>>();
  readonly #queueItemBytes: number;
  #disposed = false;
  #generationFields = 0;

  constructor(options: UploadManagerOptions) {
    if (
      !validLimit(options.chunkBytes) ||
      !validLimit(options.maxActive) ||
      !validLimit(options.maxItems) ||
      !validLimit(options.maxQueueBytes) ||
      options.chunkBytes > MAX_UPLOAD_CHUNK_BYTES ||
      options.maxActive > MAX_UPLOAD_ACTIVE_TRANSFERS ||
      options.maxItems > MAX_UPLOAD_FILES_PER_DOCUMENT ||
      options.maxActive > options.maxItems ||
      options.maxQueueBytes > MAX_UPLOAD_QUEUE_BYTES ||
      options.maxQueueBytes < options.maxItems
    ) {
      throw new RangeError("upload_manager_limits_invalid");
    }
    this.#options = Object.freeze({ ...options });
    this.#cleanupOwner = new UploadCleanupOwner(options.maxItems);
    this.#owner = new BoundedOwner<UploadTransfer>({
      maxActive: options.maxActive,
      maxBytes: options.maxQueueBytes,
      maxItems: options.maxItems,
    });
    this.#queueItemBytes = Math.floor(options.maxQueueBytes / options.maxItems);
    if (
      options.resourceObserver !== undefined &&
      (typeof options.resourceObserver.resources !== "function" ||
        typeof options.resourceObserver.progressApplicationCompleted !== "function" ||
        typeof options.resourceObserver.progressApplicationStarted !== "function")
    ) {
      throw new TypeError("upload_resource_observer_invalid");
    }
  }

  async select(selection: UploadSelection, files: readonly File[]): Promise<void> {
    if (this.#disposed) return;
    validateUploadField(selection.field);
    if (selection.input.type.toLowerCase() !== "file") throw new Error("upload_input_invalid");
    const selected = [...files];
    if (selected.length === 0) return;
    const generation = this.#replaceGeneration(selection.island, selection.field);
    const existing = this.#binding(selection.island, selection.field);
    const invalidCount =
      selected.length > this.#options.maxItems ||
      (!selection.input.multiple && selected.length !== 1);
    const replaceable = existing?.transfers.length ?? 0;
    if (
      invalidCount ||
      selected.length > this.#options.maxItems - this.#entries.size + replaceable
    ) {
      if (existing !== null) await this.#retireBinding(existing, true);
      if (!this.#isCurrentGeneration(selection.island, selection.field, generation)) return;
      this.#clearNativeSelection(selection.input);
      this.#safeProposal(selection.island, selection.field, null);
      this.#invalidateGeneration(selection.island, selection.field);
      return;
    }
    if (existing !== null) await this.#retireBinding(existing, false);
    if (!this.#isCurrentGeneration(selection.island, selection.field, generation)) return;
    const binding: Binding = {
      field: selection.field,
      input: selection.input,
      island: selection.island,
      multiple: selection.input.multiple,
      transfers: [],
    };
    this.#islandBindings(selection.island).set(selection.field, binding);
    for (const file of selected) this.#add(binding, file);
    this.#pump();
    await Promise.all(binding.transfers.map((entry) => this.#settled(entry)));
  }

  async cancel(island: UploadIslandPort, field: string): Promise<void> {
    this.#invalidateGeneration(island, field);
    const binding = this.#binding(island, field);
    if (binding === null) return;
    for (const entry of binding.transfers) {
      entry.permit?.dispose();
      entry.permit = null;
      entry.settle?.();
    }
    await Promise.all(binding.transfers.map(async ({ transfer }) => transfer.cancel()));
    this.#propose(binding);
  }

  async reacquire(selection: UploadSelection, file: File, handle: UploadHandle): Promise<void> {
    if (this.#disposed) return;
    validateUploadField(selection.field);
    if (selection.input.type.toLowerCase() !== "file") throw new Error("upload_input_invalid");
    const generation = this.#replaceGeneration(selection.island, selection.field);
    let reacquired: ReacquiredTransfer;
    try {
      reacquired = await reacquireUpload(this.#options.application, {
        field: selection.field,
        file,
        handle,
      });
    } catch (error: unknown) {
      if (this.#isCurrentGeneration(selection.island, selection.field, generation)) {
        this.#invalidateGeneration(selection.island, selection.field);
      }
      throw error;
    }
    if (this.#generationRetired(selection.island, selection.field, generation)) {
      return;
    }
    const existing = this.#binding(selection.island, selection.field);
    if (existing !== null) await this.#retireBinding(existing, false);
    if (this.#generationRetired(selection.island, selection.field, generation)) {
      return;
    }
    const binding: Binding = {
      field: selection.field,
      input: selection.input,
      island: selection.island,
      multiple: false,
      transfers: [],
    };
    this.#islandBindings(selection.island).set(selection.field, binding);
    this.#add(binding, file, reacquired);
    this.#pump();
    const entry = binding.transfers[0];
    if (entry !== undefined) await this.#settled(entry);
  }

  async retry(island: UploadIslandPort, field: string): Promise<void> {
    const binding = this.#binding(island, field);
    if (binding === null || this.#disposed) return;
    this.#replaceGeneration(island, field);
    const retrying = binding.transfers.filter(({ transfer, work }) => {
      const state = transfer.snapshot().state;
      return work === null && (state === "interrupted" || state === "failed");
    });
    for (const entry of retrying) this.#queue(entry);
    this.#pump();
    await Promise.all(retrying.map((entry) => this.#settled(entry)));
  }

  async remove(island: UploadIslandPort, field: string): Promise<void> {
    this.#invalidateGeneration(island, field);
    const binding = this.#binding(island, field);
    if (binding === null) return;
    await this.#retireBinding(binding, true);
  }

  observeIsland(island: UploadIslandPort, observer: UploadManagerObserver): VoidFunction {
    if (this.#disposed || typeof observer !== "function") {
      throw new Error("upload_manager_observer_invalid");
    }
    let observers = this.#islandObservers(island);
    if (observers === null) {
      if (this.#observers.size >= this.#options.maxItems) {
        throw new Error("upload_manager_observer_limit");
      }
      observers = new Set<UploadManagerObserver>();
      this.#observers.set(island, observers);
    }
    if (observers.size >= this.#options.maxItems) {
      throw new Error("upload_manager_observer_limit");
    }
    observers.add(observer);
    this.#notifyResources();
    this.#notifyObserver(observer, this.islandSnapshot(island));
    let active = true;
    return () => {
      if (!active) return;
      active = false;
      for (const [candidate, candidates] of this.#observers) {
        if (!sameIsland(candidate, island)) continue;
        candidates.delete(observer);
        if (candidates.size === 0) this.#observers.delete(candidate);
      }
      this.#notifyResources();
    };
  }

  islandSnapshot(island: UploadIslandPort, field?: string): UploadManagerSnapshot {
    if (field !== undefined) validateUploadField(field);
    const binding = this.#bindingsFor(island);
    return Object.freeze({
      cleanupObligations: this.#cleanupOwner.size(),
      uploads: Object.freeze(
        [...(binding?.values() ?? [])]
          .filter((candidate) => field === undefined || candidate.field === field)
          .flatMap((candidate) => candidate.transfers.map(({ transfer }) => transfer.snapshot())),
      ),
    });
  }

  activeFields(island: UploadIslandPort): readonly string[] {
    return Object.freeze([...(this.#bindingsFor(island)?.keys() ?? [])]);
  }

  retireIncompatible(
    island: UploadIslandPort,
    compatibleFields: readonly string[],
  ): readonly string[] {
    if (compatibleFields.length > this.#options.maxItems) {
      throw new Error("upload_manager_compatible_field_limit");
    }
    const compatible = new Set<string>();
    for (const field of compatibleFields) {
      validateUploadField(field);
      compatible.add(field);
    }
    const retired: string[] = [];
    for (const binding of [...(this.#bindingsFor(island)?.values() ?? [])]) {
      if (compatible.has(binding.field)) continue;
      this.#invalidateGeneration(island, binding.field);
      this.#dropBinding(binding, true);
      retired.push(binding.field);
    }
    return Object.freeze(retired);
  }

  retireIsland(island: UploadIslandPort): void {
    this.#retireGenerations(island);
    for (const [candidate, bindings] of [...this.#bindings]) {
      if (!sameIsland(candidate, island)) continue;
      for (const binding of bindings.values()) this.#dropBinding(binding, true);
      this.#bindings.delete(candidate);
    }
    for (const candidate of [...this.#observers.keys()]) {
      if (sameIsland(candidate, island)) this.#observers.delete(candidate);
    }
  }

  suspend(): void {
    if (this.#disposed) return;
    this.#owner.suspend();
  }

  resume(): void {
    if (this.#disposed) return;
    this.#owner.resume();
    this.#pump();
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#cleanupOwner.retire();
    for (const bindings of [...this.#bindings.values()]) {
      for (const binding of [...bindings.values()]) this.#dropBinding(binding, true);
    }
    this.#owner.retire();
    this.#entries.clear();
    this.#bindings.clear();
    this.#generations.clear();
    this.#observers.clear();
    this.#generationFields = 0;
  }

  snapshot(): UploadManagerSnapshot {
    return Object.freeze({
      cleanupObligations: this.#cleanupOwner.size(),
      uploads: Object.freeze(
        [...this.#entries.values()].map(({ transfer }) => transfer.snapshot()),
      ),
    });
  }

  resourceSnapshot(): UploadManagerResourceSnapshot {
    const owner = this.#owner.snapshot();
    let bindings = 0;
    for (const fields of this.#bindings.values()) bindings += fields.size;
    let observers = 0;
    for (const candidates of this.#observers.values()) observers += candidates.size;
    let pendingChunkBuffers = 0;
    let pendingChunkBytes = 0;
    let retainedStringCodeUnits = 0;
    const transferChunks = [];
    for (const { transfer } of this.#entries.values()) {
      const resource = transfer.resourceSnapshot();
      pendingChunkBuffers += resource.pendingChunkBuffers;
      pendingChunkBytes += resource.pendingChunkBytes;
      retainedStringCodeUnits += resource.retainedStringCodeUnits;
      if (resource.handle !== null) {
        transferChunks.push(
          Object.freeze({
            handle: resource.handle,
            pendingChunkBuffers: resource.pendingChunkBuffers,
            pendingChunkBytes: resource.pendingChunkBytes,
          }),
        );
      }
    }
    transferChunks.sort((left, right) => left.handle.localeCompare(right.handle));
    return Object.freeze({
      activeLeases: owner.active,
      bindings,
      cleanupObligations: this.#cleanupOwner.size(),
      entries: this.#entries.size,
      generationFields: this.#generationFields,
      observers,
      ownedResources: owner.ownedResources,
      pendingChunkBuffers,
      pendingChunkBytes,
      queuedBytes: owner.queuedBytes,
      queuedItems: owner.queuedItems,
      retainedStringCodeUnits,
      transferChunks: Object.freeze(transferChunks),
      waitingPermits: owner.waitingPermits,
    });
  }

  hasResourceObserver(): boolean {
    return this.#options.resourceObserver !== undefined;
  }

  observeProgressApplicationStarted(): void {
    try {
      this.#options.resourceObserver?.progressApplicationStarted();
    } catch {
      // Count-only observability cannot affect upload behavior.
    }
  }

  observeProgressApplicationCompleted(): void {
    try {
      this.#options.resourceObserver?.progressApplicationCompleted();
    } catch {
      // Count-only observability cannot affect upload behavior.
    }
  }

  inspectSecrets(): UploadSecretSnapshot {
    let chunks = 0;
    let files = 0;
    let grants = 0;
    for (const { transfer } of this.#entries.values()) {
      const secrets = transfer.inspectSecrets();
      chunks += secrets.chunks;
      files += secrets.files;
      grants += secrets.grants;
    }
    return Object.freeze({ chunks, files, grants });
  }

  #add(binding: Binding, file: File, reacquired?: ReacquiredTransfer): void {
    const transfer = new UploadTransfer({
      chunkBytes: this.#options.chunkBytes,
      connectivity: this.#options.connectivity,
      field: binding.field,
      file,
      island: binding.island,
      onHandle: (handle) => {
        const current = this.#entries.get(transfer);
        if (current !== undefined) {
          current.handle = handle;
          this.#propose(binding);
        }
      },
      onChange: () => {
        this.#notify(binding.island);
      },
      randomness: this.#options.randomness,
      reacquired,
      scheduleCleanup: (cleanup) => {
        this.#cleanupOwner.schedule(cleanup);
      },
      transport: this.#options.transport,
    });
    const resource = this.#owner.track(transfer);
    const entry: Entry = {
      binding,
      handle: null,
      lease: null,
      permit: null,
      resource,
      settle: null,
      settled: null,
      transfer,
      work: null,
    };
    binding.transfers.push(entry);
    this.#entries.set(transfer, entry);
    this.#queue(entry);
    this.#notify(binding.island);
  }

  #queue(entry: Entry): void {
    entry.settled = new Promise<void>((resolve) => {
      entry.settle = resolve;
    });
    const admission = this.#owner.enqueue(entry.transfer, this.#queueItemBytes);
    if (admission !== "accepted") {
      entry.transfer.dispose();
      entry.resource.dispose();
      entry.binding.transfers.splice(entry.binding.transfers.indexOf(entry), 1);
      this.#entries.delete(entry.transfer);
      entry.settle?.();
      this.#safeProposal(entry.binding.island, entry.binding.field, null);
    }
  }

  #pump(): void {
    if (this.#disposed) return;
    for (;;) {
      const transfer = this.#owner.dequeue();
      if (transfer === null) return;
      const entry = this.#entries.get(transfer);
      if (entry === undefined) continue;
      const permit = this.#owner.requestPermit((lease) => {
        entry.lease = lease;
        const state = transfer.snapshot().state;
        const work =
          state === "interrupted" || state === "failed" ? transfer.retry() : transfer.run();
        entry.work = work.finally(() => {
          entry.work = null;
          entry.lease?.dispose();
          entry.lease = null;
          entry.settle?.();
          this.#pump();
        });
      });
      entry.permit = permit.state() === "waiting" ? permit : null;
      if (permit.state() === "items_exceeded" || permit.state() === "retired") {
        transfer.dispose();
        entry.resource.dispose();
        this.#entries.delete(transfer);
        entry.settle?.();
      }
    }
  }

  async #settled(entry: Entry): Promise<void> {
    await entry.settled;
  }

  #binding(island: UploadIslandPort, field: string): Binding | null {
    validateUploadField(field);
    for (const [candidate, bindings] of this.#bindings) {
      if (sameIsland(candidate, island)) return bindings.get(field) ?? null;
    }
    return null;
  }

  #islandBindings(island: UploadIslandPort): Map<string, Binding> {
    let bindings = this.#bindings.get(island);
    if (bindings === undefined) {
      bindings = new Map();
      this.#bindings.set(island, bindings);
    }
    return bindings;
  }

  #bindingsFor(island: UploadIslandPort): Map<string, Binding> | null {
    for (const [candidate, bindings] of this.#bindings) {
      if (sameIsland(candidate, island)) return bindings;
    }
    return null;
  }

  #islandObservers(island: UploadIslandPort): Set<UploadManagerObserver> | null {
    for (const [candidate, observers] of this.#observers) {
      if (sameIsland(candidate, island)) return observers;
    }
    return null;
  }

  #notify(island: UploadIslandPort): void {
    this.#notifyResources();
    const observers = this.#islandObservers(island);
    if (observers === null || observers.size === 0) return;
    const snapshot = this.islandSnapshot(island);
    for (const observer of [...observers]) this.#notifyObserver(observer, snapshot);
  }

  #notifyResources(): void {
    try {
      this.#options.resourceObserver?.resources(this.resourceSnapshot());
    } catch {
      // Count-only observability cannot affect upload behavior.
    }
  }

  #notifyObserver(observer: UploadManagerObserver, snapshot: UploadManagerSnapshot): void {
    try {
      observer(snapshot);
    } catch {
      // Presentation observation cannot change transfer ownership.
    }
  }

  #generationMap(island: UploadIslandPort): Map<string, object> | null {
    for (const [candidate, generations] of this.#generations) {
      if (sameIsland(candidate, island)) return generations;
    }
    return null;
  }

  #replaceGeneration(island: UploadIslandPort, field: string): object {
    let generations = this.#generationMap(island);
    if (generations?.has(field) !== true && this.#generationFields >= this.#options.maxItems) {
      throw new Error("upload_manager_generation_limit");
    }
    if (generations === null) {
      generations = new Map<string, object>();
      this.#generations.set(island, generations);
    }
    if (!generations.has(field)) {
      this.#generationFields += 1;
    }
    const generation = Object.freeze({});
    generations.set(field, generation);
    return generation;
  }

  #isCurrentGeneration(island: UploadIslandPort, field: string, generation: object): boolean {
    return this.#generationMap(island)?.get(field) === generation;
  }

  #generationRetired(island: UploadIslandPort, field: string, generation: object): boolean {
    return this.#disposed || !this.#isCurrentGeneration(island, field, generation);
  }

  #invalidateGeneration(island: UploadIslandPort, field: string): void {
    for (const [candidate, generations] of this.#generations) {
      if (!sameIsland(candidate, island)) continue;
      if (generations.delete(field)) this.#generationFields -= 1;
      if (generations.size === 0) this.#generations.delete(candidate);
    }
  }

  #retireGenerations(island: UploadIslandPort): void {
    for (const [candidate, generations] of [...this.#generations]) {
      if (!sameIsland(candidate, island)) continue;
      this.#generationFields -= generations.size;
      this.#generations.delete(candidate);
    }
  }

  async #retireBinding(binding: Binding, clearInput: boolean): Promise<void> {
    await Promise.all(binding.transfers.map(async ({ transfer }) => transfer.cancel()));
    this.#dropBinding(binding, clearInput);
  }

  #dropBinding(binding: Binding, clearInput: boolean): void {
    for (const entry of binding.transfers) {
      entry.permit?.dispose();
      entry.lease?.dispose();
      entry.settle?.();
      void entry.transfer.cancel();
      entry.resource.dispose();
      this.#entries.delete(entry.transfer);
    }
    binding.transfers.length = 0;
    this.#bindings.get(binding.island)?.delete(binding.field);
    if (clearInput) this.#clearNativeSelection(binding.input);
    this.#safeProposal(binding.island, binding.field, null);
    this.#notify(binding.island);
  }

  #propose(binding: Binding): void {
    const handles = binding.transfers.flatMap(({ handle }) => (handle === null ? [] : [handle]));
    const proposal =
      handles.length === 0
        ? null
        : binding.multiple
          ? Object.freeze(handles)
          : (handles[0] ?? null);
    binding.island.proposeUploadHandle(binding.field, proposal);
  }

  #safeProposal(island: UploadIslandPort, field: string, proposal: null): void {
    try {
      island.proposeUploadHandle(field, proposal);
    } catch {
      // Core rejects invalid or retired proposal contexts without retaining feature authority.
    }
  }

  #clearNativeSelection(input: HTMLInputElement): void {
    try {
      input.value = "";
    } catch {
      // Clearing is the only allowed native file assignment and remains best-effort.
    }
  }
}
