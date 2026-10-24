export const UPLOAD_BUDGET_OBSERVER_MARKER = "suprnova-upload-budget-observer-v1";

const FEATURE_FORMAT = Symbol.for("suprnova.live.feature.v1");
const FEATURE_CORE_RANGE = 1_099_511_758_848;
const FORBIDDEN_PRODUCTION_MODULE = /(?:^|\/)src\/uploads\/(?:manager|progress|transfer)\.ts$/u;

/** Count-only snapshot emitted by the production upload manager. */
export interface ObservedUploadManagerResources {
  readonly activeLeases: number;
  readonly bindings: number;
  readonly cleanupObligations: number;
  readonly entries: number;
  readonly generationFields: number;
  readonly observers: number;
  readonly ownedResources: number;
  readonly pendingChunkBuffers: number;
  readonly pendingChunkBytes: number;
  readonly queuedBytes: number;
  readonly queuedItems: number;
  readonly retainedStringCodeUnits: number;
  readonly transferChunks: readonly Readonly<{
    readonly pendingChunkBuffers: number;
    readonly pendingChunkBytes: number;
    readonly slot: number;
  }>[];
  readonly waitingPermits: number;
}

export type UploadManagerAccountingCategories = Omit<
  ObservedUploadManagerResources,
  "transferChunks"
>;

export interface ObservedTransferChunkOwnership {
  readonly buffers: number;
  readonly bytes: number;
  readonly slot: number;
}

export interface ObservedTransferChunkHighWater {
  readonly currentBytes: number;
  readonly currentManagerBuffers: number;
  readonly currentTotalBuffers: number;
  readonly currentTransportBuffers: number;
  readonly managerHighWater: number;
  readonly managerHighWaterBytes: number;
  readonly totalHighWater: number;
  readonly totalHighWaterBytes: number;
  readonly transportHighWater: number;
  readonly transportHighWaterBytes: number;
  readonly slot: number;
}

export interface UploadTransferChunkObservation {
  readonly chunkBuffersByTransfer: readonly ObservedTransferChunkHighWater[];
  readonly liveChunkBuffers: number;
  readonly managerChunkBuffers: number;
  readonly maxChunksPerTransfer: number;
  readonly transportChunkBuffers: number;
}

function ownershipBySlot(
  ownership: readonly ObservedTransferChunkOwnership[],
): ReadonlyMap<number, ObservedTransferChunkOwnership> {
  const bySlot = new Map<number, ObservedTransferChunkOwnership>();
  for (const entry of ownership) {
    if (
      !Number.isSafeInteger(entry.slot) ||
      entry.slot < 0 ||
      !Number.isSafeInteger(entry.buffers) ||
      entry.buffers < 0 ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes < 0 ||
      bySlot.has(entry.slot)
    ) {
      throw new Error("upload_budget_transfer_chunk_ownership_invalid");
    }
    bySlot.set(entry.slot, entry);
  }
  return bySlot;
}

/** Tracks exact current and per-transfer high-water ownership without averaging. */
export class UploadTransferChunkObserver {
  readonly #currentAtDocumentHigh = new Map<
    number,
    Readonly<{
      currentBytes: number;
      currentManagerBuffers: number;
      currentTotalBuffers: number;
      currentTransportBuffers: number;
    }>
  >();
  readonly #highs = new Map<number, ObservedTransferChunkHighWater>();
  #liveChunkBuffers = 0;
  #managerChunkBuffers = 0;
  #transportChunkBuffers = 0;

  observe(
    managerOwnership: readonly ObservedTransferChunkOwnership[],
    transportOwnership: readonly ObservedTransferChunkOwnership[],
  ): void {
    const manager = ownershipBySlot(managerOwnership);
    const transport = ownershipBySlot(transportOwnership);
    const managerChunkBuffers = [...manager.values()].reduce(
      (sum, entry) => sum + entry.buffers,
      0,
    );
    const transportChunkBuffers = [...transport.values()].reduce(
      (sum, entry) => sum + entry.buffers,
      0,
    );
    const liveChunkBuffers = managerChunkBuffers + transportChunkBuffers;
    if (liveChunkBuffers >= this.#liveChunkBuffers) {
      this.#liveChunkBuffers = liveChunkBuffers;
      this.#managerChunkBuffers = managerChunkBuffers;
      this.#transportChunkBuffers = transportChunkBuffers;
      this.#currentAtDocumentHigh.clear();
      for (const slot of new Set([...manager.keys(), ...transport.keys()])) {
        const managerEntry = manager.get(slot);
        const transportEntry = transport.get(slot);
        this.#currentAtDocumentHigh.set(
          slot,
          Object.freeze({
            currentBytes: (managerEntry?.bytes ?? 0) + (transportEntry?.bytes ?? 0),
            currentManagerBuffers: managerEntry?.buffers ?? 0,
            currentTotalBuffers: (managerEntry?.buffers ?? 0) + (transportEntry?.buffers ?? 0),
            currentTransportBuffers: transportEntry?.buffers ?? 0,
          }),
        );
      }
    }
    for (const slot of new Set([...manager.keys(), ...transport.keys()])) {
      const managerBuffers = manager.get(slot)?.buffers ?? 0;
      const managerBytes = manager.get(slot)?.bytes ?? 0;
      const transportBuffers = transport.get(slot)?.buffers ?? 0;
      const transportBytes = transport.get(slot)?.bytes ?? 0;
      const prior = this.#highs.get(slot);
      this.#highs.set(
        slot,
        Object.freeze({
          currentBytes: 0,
          currentManagerBuffers: 0,
          currentTotalBuffers: 0,
          currentTransportBuffers: 0,
          managerHighWater: Math.max(prior?.managerHighWater ?? 0, managerBuffers),
          managerHighWaterBytes: Math.max(prior?.managerHighWaterBytes ?? 0, managerBytes),
          totalHighWater: Math.max(prior?.totalHighWater ?? 0, managerBuffers + transportBuffers),
          totalHighWaterBytes: Math.max(
            prior?.totalHighWaterBytes ?? 0,
            managerBytes + transportBytes,
          ),
          transportHighWater: Math.max(prior?.transportHighWater ?? 0, transportBuffers),
          transportHighWaterBytes: Math.max(prior?.transportHighWaterBytes ?? 0, transportBytes),
          slot,
        }),
      );
    }
  }

  snapshot(): UploadTransferChunkObservation {
    const chunkBuffersByTransfer = Object.freeze(
      [...this.#highs.values()]
        .map((high) =>
          Object.freeze({
            ...high,
            ...(this.#currentAtDocumentHigh.get(high.slot) ?? {
              currentBytes: 0,
              currentManagerBuffers: 0,
              currentTotalBuffers: 0,
              currentTransportBuffers: 0,
            }),
          }),
        )
        .sort((left, right) => left.slot - right.slot),
    );
    return Object.freeze({
      chunkBuffersByTransfer,
      liveChunkBuffers: this.#liveChunkBuffers,
      managerChunkBuffers: this.#managerChunkBuffers,
      maxChunksPerTransfer: Math.max(
        0,
        ...chunkBuffersByTransfer.map(({ totalHighWater }) => totalHighWater),
      ),
      transportChunkBuffers: this.#transportChunkBuffers,
    });
  }
}

export interface UploadArtifactNamespace {
  readonly configureUploads: (options: Readonly<Record<string, unknown>>) => void;
  readonly uploadsFeature: readonly [
    symbol,
    0,
    1,
    number,
    object,
    (event: number, value: unknown) => boolean,
  ];
}

/**
 * Conservative deterministic accounting model for manager-owned JavaScript data.
 * Files and pending chunk ArrayBuffers are excluded and reported separately.
 * Fixed records use 128 bytes, collection entries 64 bytes, live promises and
 * permit/lease records 256 bytes, and strings two bytes per UTF-16 code unit.
 */
export function estimateUploadManagerOwnedBytes(
  snapshot: UploadManagerAccountingCategories,
): number {
  const managerAndOwnerRecords = 1_024;
  const entryAndTransferRecords = snapshot.entries * 1_536;
  const bindingRecords = snapshot.bindings * 512;
  const collectionEntries =
    (snapshot.entries +
      snapshot.bindings +
      snapshot.generationFields +
      snapshot.observers +
      snapshot.ownedResources) *
    64;
  const queueMetadata = snapshot.queuedItems * 256;
  const permitMetadata = (snapshot.activeLeases + snapshot.waitingPermits) * 256;
  const cleanupMetadata = snapshot.cleanupObligations * 256;
  const stringStorage = snapshot.retainedStringCodeUnits * 2;
  return (
    managerAndOwnerRecords +
    entryAndTransferRecords +
    bindingRecords +
    collectionEntries +
    queueMetadata +
    permitMetadata +
    cleanupMetadata +
    stringStorage
  );
}

export function assertUploadBenchmarkBundleInputs(inputs: readonly string[]): void {
  if (inputs.some((input) => FORBIDDEN_PRODUCTION_MODULE.test(input.replace(/\\/gu, "/")))) {
    throw new Error("upload_budget_bundle_contains_production_implementation");
  }
}

export function assertUploadArtifactNamespace(
  value: unknown,
): asserts value is UploadArtifactNamespace {
  if ((typeof value !== "object" && typeof value !== "function") || value === null) {
    throw new Error("upload_budget_artifact_surface_invalid");
  }
  const candidate = value as Record<PropertyKey, unknown>;
  const feature = candidate["uploadsFeature"];
  if (
    typeof candidate["configureUploads"] !== "function" ||
    !Array.isArray(feature) ||
    !Object.isFrozen(feature) ||
    feature.length !== 6
  ) {
    throw new Error("upload_budget_artifact_surface_invalid");
  }
  const identity: unknown = feature[4];
  if (
    feature[0] !== FEATURE_FORMAT ||
    feature[1] !== 0 ||
    feature[2] !== 1 ||
    feature[3] !== FEATURE_CORE_RANGE ||
    (typeof identity !== "object" && typeof identity !== "function") ||
    identity === null ||
    !Object.isFrozen(identity) ||
    Reflect.ownKeys(identity).length !== 0 ||
    typeof feature[5] !== "function"
  ) {
    throw new Error("upload_budget_artifact_surface_invalid");
  }
}
