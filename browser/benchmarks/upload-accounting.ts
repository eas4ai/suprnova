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
  readonly waitingPermits: number;
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
export function estimateUploadManagerOwnedBytes(snapshot: ObservedUploadManagerResources): number {
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
