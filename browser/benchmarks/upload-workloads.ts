import {
  assertUploadArtifactNamespace,
  estimateUploadManagerOwnedBytes,
  type UploadManagerAccountingCategories,
  type ObservedUploadManagerResources,
  type UploadArtifactNamespace,
  UploadTransferChunkObserver,
  UPLOAD_BUDGET_OBSERVER_MARKER,
} from "./upload-accounting.js";
import { U4_16, summarizeUploadSamples } from "./upload-schema.js";

const WARMUP_PROGRESS_APPLICATIONS = 5;

interface UploadWorkloadMeasurement {
  readonly activeTransfers: number;
  readonly chunkBuffersByTransfer: readonly TransferChunkHighWater[];
  readonly liveChunkBuffers: number;
  readonly managerChunkBuffers: number;
  readonly managerOwnedBytes: number;
  readonly managerOwnedCategories: UploadManagerAccountingCategories;
  readonly maxChunksPerTransfer: number;
  readonly maxConcurrentTransfers: number;
  readonly maxQueueDepth: number;
  readonly progressP50Milliseconds: number;
  readonly progressP95Milliseconds: number;
  readonly progressDurationsMilliseconds: readonly number[];
  readonly progressSamples: number;
  readonly retainedBytes: number;
  readonly slicedBytes: number;
  readonly slices: number;
  readonly transportChunkBuffers: number;
}

interface TransferChunkHighWater {
  readonly currentBytes: number;
  readonly currentManagerBuffers: number;
  readonly currentTotalBuffers: number;
  readonly currentTransportBuffers: number;
  readonly handle: string;
  readonly managerHighWater: number;
  readonly managerHighWaterBytes: number;
  readonly totalHighWater: number;
  readonly totalHighWaterBytes: number;
  readonly transportHighWater: number;
  readonly transportHighWaterBytes: number;
}

interface FileSliceObservation {
  bytes: number;
  calls: number;
}

interface UploadTransportRequest {
  readonly bytes?: ArrayBuffer;
  readonly expectedRevision?: string;
  readonly handle?: string;
  readonly operation: "cancel" | "complete" | "create" | "put_chunk" | "status";
}

interface UploadTransportResponse {
  readonly grant?: string;
  readonly handle?: string;
  readonly nextChunkIndex?: number;
  readonly revision: string;
  readonly state: string;
}

class ObservedFile extends File {
  readonly #observation: FileSliceObservation;

  constructor(name: string, observation: FileSliceObservation) {
    super([new Uint8Array(U4_16.fileBytes)], name, {
      lastModified: 1_700_000_000_000,
      type: "application/octet-stream",
    });
    this.#observation = observation;
  }

  override slice(start?: number, end?: number, contentType?: string): Blob {
    const normalizedStart = start ?? 0;
    const normalizedEnd = end ?? this.size;
    this.#observation.calls += 1;
    this.#observation.bytes += Math.max(0, normalizedEnd - normalizedStart);
    return super.slice(start, end, contentType);
  }
}

class ImmediateUploadTransport {
  readonly #revisions = new Map<string, bigint>();
  readonly #activeChunks = new Map<string, ArrayBuffer>();
  readonly #completed: Promise<void>;
  #completeCount = 0;
  #created = 0;
  #maximumConcurrentTransfers = 0;
  #observeResources: (() => void) | null = null;
  #resolveCompleted!: () => void;

  constructor() {
    this.#completed = new Promise<void>((resolve) => {
      this.#resolveCompleted = resolve;
    });
  }

  observeResources(observer: () => void): void {
    this.#observeResources = observer;
  }

  activeChunkBytes(): number {
    let bytes = 0;
    for (const chunk of this.#activeChunks.values()) bytes += chunk.byteLength;
    return bytes;
  }

  activeChunkBuffers(): number {
    return this.#activeChunks.size;
  }

  activeChunksByTransfer(): readonly Readonly<{
    bytes: number;
    buffers: number;
    handle: string;
  }>[] {
    return Object.freeze(
      [...this.#activeChunks.entries()]
        .map(([handle, bytes]) => Object.freeze({ bytes: bytes.byteLength, buffers: 1, handle }))
        .sort((left, right) => left.handle.localeCompare(right.handle)),
    );
  }

  completed(): Promise<void> {
    return this.#completed;
  }

  maximumConcurrentTransfers(): number {
    return this.#maximumConcurrentTransfers;
  }

  async send(request: UploadTransportRequest): Promise<UploadTransportResponse> {
    switch (request.operation) {
      case "create": {
        this.#created += 1;
        const suffix = String(this.#created).padStart(12, "0");
        const handle = `018f47c1-2af0-7cc4-a001-${suffix}`;
        this.#revisions.set(handle, 1n);
        return Object.freeze({
          grant: `benchmark-grant-${String(this.#created)}`,
          handle,
          revision: "1",
          state: "queued",
        });
      }
      case "put_chunk": {
        if (request.handle === undefined || request.bytes === undefined) {
          throw new Error("upload_budget_request_invalid");
        }
        const current = this.#revisions.get(request.handle);
        if (current === undefined || current.toString() !== request.expectedRevision) {
          throw new Error("upload_budget_revision_mismatch");
        }
        this.#activeChunks.set(request.handle, request.bytes);
        this.#maximumConcurrentTransfers = Math.max(
          this.#maximumConcurrentTransfers,
          this.#activeChunks.size,
        );
        this.#observeResources?.();
        await Promise.resolve();
        const next = current + 1n;
        this.#revisions.set(request.handle, next);
        this.#activeChunks.delete(request.handle);
        this.#observeResources?.();
        return Object.freeze({ revision: next.toString(), state: "transferring" });
      }
      case "complete": {
        if (request.handle === undefined) throw new Error("upload_budget_request_invalid");
        const current = this.#revisions.get(request.handle);
        if (current === undefined || current.toString() !== request.expectedRevision) {
          throw new Error("upload_budget_revision_mismatch");
        }
        const next = current + 1n;
        this.#revisions.set(request.handle, next);
        this.#completeCount += 1;
        if (this.#completeCount === U4_16.files) this.#resolveCompleted();
        return Object.freeze({ revision: next.toString(), state: "ready" });
      }
      case "status": {
        if (request.handle === undefined) throw new Error("upload_budget_request_invalid");
        const current = this.#revisions.get(request.handle);
        if (current === undefined) throw new Error("upload_budget_handle_missing");
        return Object.freeze({
          nextChunkIndex: 0,
          revision: current.toString(),
          state: "transferring",
        });
      }
      case "cancel": {
        if (request.handle === undefined) throw new Error("upload_budget_request_invalid");
        const current = this.#revisions.get(request.handle);
        if (current === undefined) throw new Error("upload_budget_handle_missing");
        const next = current + 1n;
        this.#revisions.set(request.handle, next);
        return Object.freeze({ revision: next.toString(), state: "canceled" });
      }
    }
  }
}

function filesOn(input: HTMLInputElement, files: readonly File[]): void {
  Object.defineProperty(input, "files", {
    configurable: true,
    value: Object.freeze([...files]),
  });
}

function blankResources(): ObservedUploadManagerResources {
  return Object.freeze({
    activeLeases: 0,
    bindings: 0,
    cleanupObligations: 0,
    entries: 0,
    generationFields: 0,
    observers: 0,
    ownedResources: 0,
    pendingChunkBuffers: 0,
    pendingChunkBytes: 0,
    queuedBytes: 0,
    queuedItems: 0,
    retainedStringCodeUnits: 0,
    transferChunks: Object.freeze([]),
    waitingPermits: 0,
  });
}

function accountingCategories(
  resources: ObservedUploadManagerResources,
): UploadManagerAccountingCategories {
  return Object.freeze({
    activeLeases: resources.activeLeases,
    bindings: resources.bindings,
    cleanupObligations: resources.cleanupObligations,
    entries: resources.entries,
    generationFields: resources.generationFields,
    observers: resources.observers,
    ownedResources: resources.ownedResources,
    pendingChunkBuffers: resources.pendingChunkBuffers,
    pendingChunkBytes: resources.pendingChunkBytes,
    queuedBytes: resources.queuedBytes,
    queuedItems: resources.queuedItems,
    retainedStringCodeUnits: resources.retainedStringCodeUnits,
    waitingPermits: resources.waitingPermits,
  });
}

/** Executes U4/16 only through the exact imported production artifact surface. */
export async function measureU4_16(artifactValue: unknown): Promise<UploadWorkloadMeasurement> {
  assertUploadArtifactNamespace(artifactValue);
  const artifact: UploadArtifactNamespace = artifactValue;
  const islandElement = document.createElement("section");
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = true;
  input.setAttribute("live:upload", "attachments");
  const progress = document.createElement("div");
  progress.setAttribute("live:progress", "attachments");
  islandElement.append(input, progress);
  document.body.replaceChildren(islandElement);

  const proposals: unknown[] = [];
  const transport = new ImmediateUploadTransport();
  const progressSamples: number[] = [];
  let progressApplications = 0;
  let progressApplicationStartedAt = 0;
  let latestResources = blankResources();
  let managerOwnedBytes = 0;
  let managerOwnedCategories = accountingCategories(latestResources);
  const chunkObserver = new UploadTransferChunkObserver();
  let maxConcurrentTransfers = 0;
  let maxQueueDepth = 0;
  let retainedBytes = 0;

  const observeResources = (): void => {
    const managerBytes = estimateUploadManagerOwnedBytes(latestResources);
    if (managerBytes >= managerOwnedBytes) {
      managerOwnedBytes = managerBytes;
      managerOwnedCategories = accountingCategories(latestResources);
    }
    chunkObserver.observe(
      latestResources.transferChunks.map((transfer) => ({
        buffers: transfer.pendingChunkBuffers,
        bytes: transfer.pendingChunkBytes,
        handle: transfer.handle,
      })),
      transport.activeChunksByTransfer(),
    );
    maxConcurrentTransfers = Math.max(
      maxConcurrentTransfers,
      latestResources.activeLeases,
      transport.maximumConcurrentTransfers(),
    );
    maxQueueDepth = Math.max(maxQueueDepth, latestResources.queuedItems);
    retainedBytes = Math.max(
      retainedBytes,
      managerBytes + latestResources.pendingChunkBytes + transport.activeChunkBytes(),
    );
  };
  transport.observeResources(observeResources);
  artifact.configureUploads({
    chunkBytes: U4_16.chunkBytes,
    connectivity: Object.freeze({ online: () => true }),
    maxActive: U4_16.activeTransfers,
    maxItems: U4_16.files,
    maxQueueBytes: 256 * 1024,
    randomness: Object.freeze({
      idempotencyKey: (() => {
        let next = 0;
        return () => {
          next += 1;
          return `u4-16-${String(next)}`;
        };
      })(),
    }),
    resourceObserver: Object.freeze({
      progressApplicationCompleted() {
        const durationMilliseconds = performance.now() - progressApplicationStartedAt;
        progressApplications += 1;
        if (progressApplications > WARMUP_PROGRESS_APPLICATIONS) {
          progressSamples.push(durationMilliseconds);
        }
      },
      progressApplicationStarted() {
        progressApplicationStartedAt = performance.now();
      },
      resources(snapshot: ObservedUploadManagerResources) {
        latestResources = snapshot;
        observeResources();
      },
    }),
    transport,
  });

  const drive = artifact.uploadsFeature[5];
  const documentConnected = drive(
    0,
    Object.freeze({
      diagnose() {
        return undefined;
      },
      onDispose() {
        return undefined;
      },
    }),
  );
  const islandConnected = drive(
    1,
    Object.freeze({
      element: islandElement,
      identity: Object.freeze({
        component: "benchmark.uploads",
        documentKey: "u4-16",
        slot: "uploads",
      }),
      proposeUploadHandle(_field: string, proposal: unknown) {
        proposals.push(proposal);
        return "accepted";
      },
    }),
  );
  if (!documentConnected || !islandConnected)
    throw new Error("upload_budget_artifact_drive_failed");

  const slices: FileSliceObservation = { bytes: 0, calls: 0 };
  const files = Array.from(
    { length: U4_16.files },
    (_, index) => new ObservedFile(`u4-16-${String(index)}.bin`, slices),
  );
  try {
    filesOn(input, files);
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await transport.completed();
    for (
      let turn = 0;
      turn < 64 && progress.getAttribute("data-live-upload-state") !== "ready";
      turn += 1
    ) {
      await Promise.resolve();
    }
    observeResources();
    if (
      progress.getAttribute("data-live-upload-state") !== "ready" ||
      progress.getAttribute("data-live-upload-loaded") !== String(U4_16.files * U4_16.fileBytes) ||
      proposals.length < U4_16.files ||
      slices.calls !== U4_16.files * (U4_16.fileBytes / U4_16.chunkBytes) ||
      slices.bytes !== U4_16.files * U4_16.fileBytes ||
      progressSamples.length < 30 ||
      maxConcurrentTransfers !== U4_16.activeTransfers
    ) {
      throw new Error("upload_budget_workload_incomplete");
    }
    const progressSummary = summarizeUploadSamples(progressSamples);
    const chunks = chunkObserver.snapshot();
    const chunkBuffersByTransfer = chunks.chunkBuffersByTransfer;
    if (chunkBuffersByTransfer.length !== U4_16.activeTransfers) {
      throw new Error("upload_budget_transfer_buffer_evidence_incomplete");
    }
    return Object.freeze({
      activeTransfers: U4_16.activeTransfers,
      chunkBuffersByTransfer,
      liveChunkBuffers: chunks.liveChunkBuffers,
      managerChunkBuffers: chunks.managerChunkBuffers,
      managerOwnedBytes,
      managerOwnedCategories,
      maxChunksPerTransfer: chunks.maxChunksPerTransfer,
      maxConcurrentTransfers,
      maxQueueDepth,
      progressP50Milliseconds: progressSummary.p50,
      progressP95Milliseconds: progressSummary.p95,
      progressDurationsMilliseconds: Object.freeze([...progressSamples]),
      progressSamples: progressSamples.length,
      retainedBytes,
      slicedBytes: slices.bytes,
      slices: slices.calls,
      transportChunkBuffers: chunks.transportChunkBuffers,
    });
  } finally {
    drive(4, islandElement);
    drive(5, null);
  }
}

export { UPLOAD_BUDGET_OBSERVER_MARKER };
