import { UploadManager } from "../src/uploads/manager.js";
import { createUploadProgressView, UploadProgressPresenter } from "../src/uploads/progress.js";
import type {
  UploadHandle,
  UploadHandleProposal,
  UploadIslandPort,
  UploadManagerSnapshot,
  UploadTransport,
  UploadTransportRequest,
  UploadTransportResponse,
} from "../src/uploads/types.js";
import { U4_16, summarizeUploadSamples } from "./upload-schema.js";

const MANAGER_BYTES = 256 * 1024;
const WARMUP_PROGRESS_APPLICATIONS = 5;
export const UPLOAD_BUDGET_OBSERVER_MARKER = "suprnova-upload-budget-observer-v1";

interface UploadWorkloadMeasurement {
  readonly activeTransfers: number;
  readonly liveChunkBuffers: number;
  readonly managerOwnedBytes: number;
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
}

interface FileSliceObservation {
  bytes: number;
  calls: number;
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

class ImmediateUploadTransport implements UploadTransport {
  readonly #revisions = new Map<UploadHandle, bigint>();
  readonly #activeChunks = new Map<UploadHandle, ArrayBuffer>();
  #created = 0;
  #maximumConcurrentTransfers = 0;
  #observeResources: (() => void) | null = null;

  observeResources(observer: () => void): void {
    this.#observeResources = observer;
  }

  activeChunkBuffers(): ReadonlyMap<UploadHandle, ArrayBuffer> {
    return this.#activeChunks;
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
        const current = this.#revisions.get(request.handle);
        if (current?.toString() !== request.expectedRevision) {
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
        const current = this.#revisions.get(request.handle);
        if (current?.toString() !== request.expectedRevision) {
          throw new Error("upload_budget_revision_mismatch");
        }
        const next = current + 1n;
        this.#revisions.set(request.handle, next);
        return Object.freeze({ revision: next.toString(), state: "ready" });
      }
      case "status": {
        const current = this.#revisions.get(request.handle);
        if (current === undefined) throw new Error("upload_budget_handle_missing");
        return Object.freeze({
          nextChunkIndex: 0,
          revision: current.toString(),
          state: "transferring",
        });
      }
      case "cancel": {
        const current = this.#revisions.get(request.handle);
        if (current === undefined) throw new Error("upload_budget_handle_missing");
        const next = current + 1n;
        this.#revisions.set(request.handle, next);
        return Object.freeze({ revision: next.toString(), state: "canceled" });
      }
    }
  }
}

function chunkHighWater(
  snapshot: UploadManagerSnapshot,
  active: ReadonlyMap<UploadHandle, ArrayBuffer>,
): Readonly<{ live: number; perTransfer: number }> {
  let live = active.size;
  let perTransfer = active.size === 0 ? 0 : 1;
  for (const transfer of snapshot.uploads) {
    live += transfer.retainedChunks;
    const transport = transfer.handle === null || !active.has(transfer.handle) ? 0 : 1;
    perTransfer = Math.max(perTransfer, transfer.retainedChunks + transport);
  }
  return Object.freeze({ live, perTransfer });
}

/**
 * Runs the exact U4/16 browser path through production manager, File slicing,
 * checksum, transport validation, and progress presentation code. This module
 * is compiled only into the benchmark harness and is never a production entry.
 */
export async function measureU4_16(): Promise<UploadWorkloadMeasurement> {
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = true;
  const islandElement = document.createElement("section");
  const progress = document.createElement("div");
  document.body.replaceChildren(islandElement, input, progress);

  const proposals: unknown[] = [];
  const island: UploadIslandPort = Object.freeze({
    element: islandElement,
    identity: Object.freeze({
      component: "benchmark.uploads",
      documentKey: "u4-16",
      slot: "uploads",
    }),
    proposeUploadHandle(_field: string, proposal: UploadHandleProposal) {
      proposals.push(proposal);
      return "accepted";
    },
  });
  let randomness = 0;
  const transport = new ImmediateUploadTransport();
  const manager = new UploadManager({
    chunkBytes: U4_16.chunkBytes,
    connectivity: { online: () => true },
    maxActive: U4_16.activeTransfers,
    maxItems: U4_16.files,
    maxQueueBytes: MANAGER_BYTES,
    randomness: {
      idempotencyKey() {
        randomness += 1;
        return `u4-16-${String(randomness)}`;
      },
    },
    transport,
  });
  const presenter = new UploadProgressPresenter({ announceEveryMs: 0 });
  const progressSamples: number[] = [];
  let progressApplications = 0;
  let maxQueueDepth = 0;
  let liveChunkBuffers = 0;
  let maxChunksPerTransfer = 0;
  let maxConcurrentTransfers = 0;

  const observeResources = (): void => {
    const snapshot = manager.islandSnapshot(island, "attachments");
    const chunks = chunkHighWater(snapshot, transport.activeChunkBuffers());
    liveChunkBuffers = Math.max(liveChunkBuffers, chunks.live);
    maxChunksPerTransfer = Math.max(maxChunksPerTransfer, chunks.perTransfer);
    maxConcurrentTransfers = Math.max(
      maxConcurrentTransfers,
      snapshot.uploads.filter(
        ({ state }) => state === "transferring" || state === "verifying" || state === "finalizing",
      ).length,
    );
    maxQueueDepth = Math.max(
      maxQueueDepth,
      snapshot.uploads.filter(({ state }) => state === "queued").length,
    );
  };
  transport.observeResources(observeResources);
  const stopObserving = manager.observeIsland(island, (snapshot) => {
    observeResources();
    const view = createUploadProgressView(snapshot.uploads);
    if (view === null) return;
    const started = performance.now();
    presenter.render(progress, view);
    const elapsed = performance.now() - started;
    progressApplications += 1;
    if (progressApplications > WARMUP_PROGRESS_APPLICATIONS) progressSamples.push(elapsed);
  });

  const slices: FileSliceObservation = { bytes: 0, calls: 0 };
  const files = Array.from(
    { length: U4_16.files },
    (_, index) => new ObservedFile(`u4-16-${String(index)}.bin`, slices),
  );
  try {
    await manager.select({ field: "attachments", input, island }, files);
    observeResources();
    const final = manager.islandSnapshot(island, "attachments");
    if (
      final.uploads.length !== U4_16.files ||
      final.uploads.some(
        ({ sentBytes, size, state }) =>
          sentBytes !== U4_16.fileBytes || size !== U4_16.fileBytes || state !== "ready",
      ) ||
      proposals.length < U4_16.files ||
      slices.calls !== U4_16.files * (U4_16.fileBytes / U4_16.chunkBytes) ||
      slices.bytes !== U4_16.files * U4_16.fileBytes ||
      progressSamples.length < 30
    ) {
      throw new Error("upload_budget_workload_incomplete");
    }
    const progressSummary = summarizeUploadSamples(progressSamples);
    return Object.freeze({
      activeTransfers: U4_16.activeTransfers,
      liveChunkBuffers,
      managerOwnedBytes: MANAGER_BYTES,
      maxChunksPerTransfer,
      maxConcurrentTransfers,
      maxQueueDepth,
      progressP50Milliseconds: progressSummary.p50,
      progressP95Milliseconds: progressSummary.p95,
      progressDurationsMilliseconds: Object.freeze([...progressSamples]),
      progressSamples: progressSamples.length,
      retainedBytes: liveChunkBuffers * U4_16.chunkBytes + MANAGER_BYTES,
      slicedBytes: slices.bytes,
      slices: slices.calls,
    });
  } finally {
    stopObserving();
    manager.dispose();
    presenter.clear(progress);
  }
}
