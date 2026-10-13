import {
  uploadFileIdentitiesEqual,
  uploadFileIdentity,
  validateTransportResponse,
  validateUploadIdempotencyKey,
  type ReacquiredTransfer,
  type SecretTransferGrant,
  type UploadConnectivity,
  type UploadHandle,
  type UploadIslandPort,
  type UploadPresentationState,
  type UploadRandomness,
  type UploadSecretSnapshot,
  type UploadTransferSnapshot,
  type UploadTransport,
  type UploadTransportRequest,
  type UploadTransportResponse,
} from "./types.js";

const SHA256_INITIAL = new Uint32Array([
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]);
const SHA256_CONSTANTS = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

interface PendingChunk {
  readonly bytes: ArrayBuffer;
  readonly checksum: string;
  readonly idempotencyKey: string;
  readonly index: number;
  readonly offset: number;
}

export interface UploadTransferOptions {
  readonly chunkBytes: number;
  readonly connectivity: UploadConnectivity;
  readonly field: string;
  readonly file: File;
  readonly island: UploadIslandPort;
  readonly onChange?: (() => void) | undefined;
  readonly onHandle?: ((handle: UploadHandle | null) => void) | undefined;
  readonly randomness: UploadRandomness;
  readonly reacquired?: ReacquiredTransfer | undefined;
  readonly scheduleCleanup?: UploadCleanupScheduler | undefined;
  readonly transport: UploadTransport;
}

export interface UploadCancellationCleanup {
  readonly request: Extract<UploadTransportRequest, { readonly operation: "cancel" }>;
  readonly transport: UploadTransport;
}

export interface UploadTransferResourceSnapshot {
  readonly pendingChunkBytes: number;
  readonly pendingChunkBuffers: number;
  readonly retainedStringCodeUnits: number;
}

export type UploadCleanupScheduler = (cleanup: UploadCancellationCleanup) => void;

const ignoreDetachedUploadCancellationFailure = (): void => undefined;

export async function settleUploadCancellation(cleanup: UploadCancellationCleanup): Promise<void> {
  try {
    const response = await cleanup.transport.send(cleanup.request);
    validateTransportResponse("cancel", response, cleanup.request.expectedRevision);
  } catch {
    // Server cleanup owns uncertain cancellation after local authority is released.
  }
}

export function detachUploadCancellation(cleanup: UploadCancellationCleanup): void {
  try {
    void cleanup.transport.send(cleanup.request).catch(ignoreDetachedUploadCancellationFailure);
  } catch {
    // Dispatch is best-effort after local authority is released.
  }
}

function rotateRight(value: number, count: number): number {
  return (value >>> count) | (value << (32 - count));
}

function sha256Block(state: Uint32Array, bytes: Uint8Array, offset: number): void {
  const words = new Uint32Array(64);
  for (let index = 0; index < 16; index += 1) {
    const start = offset + index * 4;
    words[index] =
      (((bytes[start] ?? 0) << 24) |
        ((bytes[start + 1] ?? 0) << 16) |
        ((bytes[start + 2] ?? 0) << 8) |
        (bytes[start + 3] ?? 0)) >>>
      0;
  }
  for (let index = 16; index < 64; index += 1) {
    const prior = words[index - 15] ?? 0;
    const recent = words[index - 2] ?? 0;
    const sigma0 = rotateRight(prior, 7) ^ rotateRight(prior, 18) ^ (prior >>> 3);
    const sigma1 = rotateRight(recent, 17) ^ rotateRight(recent, 19) ^ (recent >>> 10);
    words[index] = ((words[index - 16] ?? 0) + sigma0 + (words[index - 7] ?? 0) + sigma1) >>> 0;
  }
  let a = state[0] ?? 0;
  let b = state[1] ?? 0;
  let c = state[2] ?? 0;
  let d = state[3] ?? 0;
  let e = state[4] ?? 0;
  let f = state[5] ?? 0;
  let g = state[6] ?? 0;
  let h = state[7] ?? 0;
  for (let index = 0; index < 64; index += 1) {
    const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
    const choice = (e & f) ^ (~e & g);
    const temporary1 =
      (h + sum1 + choice + (SHA256_CONSTANTS[index] ?? 0) + (words[index] ?? 0)) >>> 0;
    const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
    const majority = (a & b) ^ (a & c) ^ (b & c);
    const temporary2 = (sum0 + majority) >>> 0;
    h = g;
    g = f;
    f = e;
    e = (d + temporary1) >>> 0;
    d = c;
    c = b;
    b = a;
    a = (temporary1 + temporary2) >>> 0;
  }
  state[0] = ((state[0] ?? 0) + a) >>> 0;
  state[1] = ((state[1] ?? 0) + b) >>> 0;
  state[2] = ((state[2] ?? 0) + c) >>> 0;
  state[3] = ((state[3] ?? 0) + d) >>> 0;
  state[4] = ((state[4] ?? 0) + e) >>> 0;
  state[5] = ((state[5] ?? 0) + f) >>> 0;
  state[6] = ((state[6] ?? 0) + g) >>> 0;
  state[7] = ((state[7] ?? 0) + h) >>> 0;
}

class IncrementalSha256 {
  readonly #state = new Uint32Array(SHA256_INITIAL);
  readonly #buffer = new Uint8Array(64);
  #buffered = 0;
  #byteLength = 0;

  update(input: Uint8Array): void {
    this.#byteLength += input.byteLength;
    let offset = 0;
    if (this.#buffered > 0) {
      const copied = Math.min(64 - this.#buffered, input.byteLength);
      this.#buffer.set(input.subarray(0, copied), this.#buffered);
      this.#buffered += copied;
      offset += copied;
      if (this.#buffered === 64) {
        sha256Block(this.#state, this.#buffer, 0);
        this.#buffered = 0;
      }
    }
    while (offset + 64 <= input.byteLength) {
      sha256Block(this.#state, input, offset);
      offset += 64;
    }
    if (offset < input.byteLength) {
      const remainder = input.subarray(offset);
      this.#buffer.set(remainder, 0);
      this.#buffered = remainder.byteLength;
    }
  }

  digestHex(): string {
    const state = new Uint32Array(this.#state);
    const finalLength = this.#buffered < 56 ? 64 : 128;
    const tail = new Uint8Array(finalLength);
    tail.set(this.#buffer.subarray(0, this.#buffered), 0);
    tail[this.#buffered] = 0x80;
    const high = Math.floor(this.#byteLength / 0x2000_0000);
    const low = (this.#byteLength * 8) >>> 0;
    const view = new DataView(tail.buffer);
    view.setUint32(finalLength - 8, high, false);
    view.setUint32(finalLength - 4, low, false);
    for (let offset = 0; offset < finalLength; offset += 64) sha256Block(state, tail, offset);
    return [...state].map((word) => word.toString(16).padStart(8, "0")).join("");
  }
}

function sha256Hex(bytes: ArrayBuffer): string {
  const digest = new IncrementalSha256();
  digest.update(new Uint8Array(bytes));
  return digest.digestHex();
}

function operationFailedAsExpired(error: unknown): boolean {
  return (
    (typeof error === "object" || typeof error === "function") &&
    error !== null &&
    "code" in error &&
    error.code === "upload_expired"
  );
}

export class UploadTransfer {
  readonly #chunkBytes: number;
  readonly #connectivity: UploadConnectivity;
  readonly #field: string;
  readonly #identity;
  readonly #island: UploadIslandPort;
  readonly #onChange: (() => void) | undefined;
  readonly #onHandle: ((handle: UploadHandle | null) => void) | undefined;
  readonly #randomness: UploadRandomness;
  readonly #scheduleCleanup: UploadCleanupScheduler;
  readonly #transport: UploadTransport;
  readonly #whole = new IncrementalSha256();
  #abort = new AbortController();
  #completeKey: string | null = null;
  #completionUncertain = false;
  #createKey: string;
  #file: File | null;
  #grant: SecretTransferGrant | null = null;
  #handle: UploadHandle | null = null;
  #hashedBytes = 0;
  #mustReconcile = false;
  #nextChunkIndex = 0;
  #offset = 0;
  #pending: PendingChunk | null = null;
  #proposed = false;
  #revision: string | null = null;
  #running: Promise<void> | null = null;
  #state: UploadPresentationState = "queued";
  #disposed = false;

  constructor(options: UploadTransferOptions) {
    if (!Number.isSafeInteger(options.chunkBytes) || options.chunkBytes < 1) {
      throw new RangeError("upload_chunk_bytes_invalid");
    }
    this.#chunkBytes = options.chunkBytes;
    this.#connectivity = options.connectivity;
    this.#field = options.field;
    this.#file = options.file;
    this.#identity = uploadFileIdentity(options.file);
    this.#island = options.island;
    this.#onChange = options.onChange;
    this.#onHandle = options.onHandle;
    this.#randomness = options.randomness;
    this.#scheduleCleanup = options.scheduleCleanup ?? detachUploadCancellation;
    this.#transport = options.transport;
    this.#createKey = this.#nextKey();
    if (options.reacquired !== undefined) {
      if (
        options.reacquired.file !== options.file ||
        !uploadFileIdentitiesEqual(this.#identity, options.reacquired.fileIdentity)
      ) {
        throw new Error("upload_reacquire_identity_mismatch");
      }
      this.#grant = options.reacquired.grant;
      this.#handle = options.reacquired.handle;
      this.#offset = options.reacquired.uploadedBytes;
      this.#nextChunkIndex = options.reacquired.nextChunkIndex;
      this.#revision = options.reacquired.revision;
      this.#state = options.reacquired.state;
      this.#mustReconcile = true;
    }
  }

  run(): Promise<void> {
    if (this.#running !== null) return this.#running;
    if (this.#disposed || this.#file === null) return Promise.resolve();
    const running = this.#execute().finally(() => {
      if (this.#running === running) this.#running = null;
    });
    this.#running = running;
    return running;
  }

  retry(): Promise<void> {
    if (this.#disposed || (this.#state !== "interrupted" && this.#state !== "failed")) {
      return Promise.resolve();
    }
    this.#abort = new AbortController();
    this.#setState("queued");
    return this.run();
  }

  cancel(): Promise<void> {
    if (this.#disposed || this.#state === "canceled") return Promise.resolve();
    this.#abort.abort();
    const handle = this.#handle;
    const grant = this.#grant;
    const revision = this.#revision;
    const cancellable = this.#state !== "expired";
    const idempotencyKey =
      handle === null || grant === null || revision === null ? null : this.#nextKey();
    this.#setState("canceled");
    this.#releaseAuthority();
    if (handle !== null && grant !== null && revision !== null && cancellable) {
      this.#sendCancellation(handle, grant, revision, idempotencyKey ?? this.#nextKey());
    }
    return Promise.resolve();
  }

  suspend(): void {
    if (this.#disposed || this.#running === null) return;
    this.#abort.abort();
    this.#setState("interrupted");
  }

  resume(): void {
    // Resumption is deliberate through retry; reconnect does not silently mutate server state.
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#abort.abort();
    if (this.#state !== "expired") this.#state = "canceled";
    this.#releaseAuthority();
    this.#changed();
  }

  snapshot(): UploadTransferSnapshot {
    return Object.freeze({
      field: this.#field,
      handle: this.#handle,
      name: this.#identity.name,
      retainedChunks: this.#pending === null ? 0 : 1,
      revision: this.#revision,
      sentBytes: this.#offset,
      size: this.#identity.size,
      state: this.#state,
    });
  }

  inspectSecrets(): UploadSecretSnapshot {
    return Object.freeze({
      chunks: this.#pending === null ? 0 : 1,
      files: this.#file === null ? 0 : 1,
      grants: this.#grant === null ? 0 : 1,
    });
  }

  resourceSnapshot(): UploadTransferResourceSnapshot {
    return Object.freeze({
      pendingChunkBytes: this.#pending?.bytes.byteLength ?? 0,
      pendingChunkBuffers: this.#pending === null ? 0 : 1,
      retainedStringCodeUnits:
        this.#field.length +
        this.#identity.name.length +
        this.#identity.type.length +
        (this.#grant?.length ?? 0) +
        (this.#handle?.length ?? 0) +
        (this.#revision?.length ?? 0) +
        (this.#completeKey?.length ?? 0) +
        this.#createKey.length +
        (this.#pending?.checksum.length ?? 0) +
        (this.#pending?.idempotencyKey.length ?? 0),
    });
  }

  async #execute(): Promise<void> {
    if (!this.#connectivity.online()) {
      this.#setState("interrupted");
      return;
    }
    try {
      if (this.#handle === null) await this.#create();
      if (this.#inactive()) return;
      if (!this.#proposed) {
        this.#propose(this.#requiredHandle());
        this.#proposed = true;
      }
      if (this.#mustReconcile || this.#completionUncertain) {
        const resumable = await this.#reconcile();
        if (!resumable || this.#inactive()) return;
      }
      await this.#hashRetainedPrefix();
      if (this.#inactive()) return;
      this.#setState("transferring");
      while (this.#offset < this.#identity.size) {
        await this.#sendNextChunk();
        if (this.#inactive() || this.#state !== "transferring") return;
      }
      if (this.#inactive()) return;
      await this.#complete();
    } catch (error: unknown) {
      if (this.#disposed || this.#state === "canceled") return;
      if (operationFailedAsExpired(error)) {
        this.#setState("expired");
        this.#releaseAuthority();
      } else if (
        !this.#connectivity.online() ||
        error instanceof TypeError ||
        this.#abort.signal.aborted
      ) {
        this.#setState("interrupted");
      } else {
        this.#setState("failed");
        if (!this.#proposed) this.#releaseAuthority();
      }
    }
  }

  async #create(): Promise<void> {
    const file = this.#requiredFile();
    const response = validateTransportResponse(
      "create",
      await this.#transport.send({
        field: this.#field,
        file: this.#identity,
        idempotencyKey: this.#createKey,
        island: this.#island.identity,
        operation: "create",
        signal: this.#abort.signal,
      }),
    );
    if (
      this.#disposed ||
      this.#abort.signal.aborted ||
      this.#state === "canceled" ||
      file !== this.#file
    ) {
      return;
    }
    this.#handle = response.handle ?? null;
    this.#grant = response.grant ?? null;
    this.#revision = response.revision;
    if (this.#handle === null || this.#grant === null) throw new Error("upload_create_invalid");
    this.#propose(this.#handle);
    this.#proposed = true;
    this.#changed();
  }

  async #sendNextChunk(): Promise<void> {
    const file = this.#requiredFile();
    const handle = this.#requiredHandle();
    const grant = this.#requiredGrant();
    const revision = this.#requiredRevision();
    if (this.#pending === null) {
      const end = Math.min(this.#offset + this.#chunkBytes, this.#identity.size);
      const bytes = await file.slice(this.#offset, end).arrayBuffer();
      if (this.#disposed || file !== this.#file) return;
      this.#whole.update(new Uint8Array(bytes));
      this.#hashedBytes += bytes.byteLength;
      this.#pending = Object.freeze({
        bytes,
        checksum: sha256Hex(bytes),
        idempotencyKey: this.#nextKey(),
        index: this.#nextChunkIndex,
        offset: this.#offset,
      });
      this.#changed();
    }
    const pending = this.#pending;
    const response = validateTransportResponse(
      "put_chunk",
      await this.#transport.send({
        bytes: pending.bytes,
        checksum: pending.checksum,
        chunkIndex: pending.index,
        expectedRevision: revision,
        grant,
        handle,
        idempotencyKey: pending.idempotencyKey,
        offset: pending.offset,
        operation: "put_chunk",
        signal: this.#abort.signal,
      }),
      revision,
    );
    if (this.#disposed || this.#abort.signal.aborted || this.#state === "canceled") return;
    if (this.#applyTerminalResponse(response)) return;
    this.#revision = response.revision;
    this.#offset += pending.bytes.byteLength;
    this.#nextChunkIndex += 1;
    this.#pending = null;
    this.#setState(response.state);
  }

  async #complete(): Promise<void> {
    const expectedRevision = this.#requiredRevision();
    this.#completeKey ??= this.#nextKey();
    this.#completionUncertain = true;
    const response = validateTransportResponse(
      "complete",
      await this.#transport.send({
        expectedRevision,
        grant: this.#requiredGrant(),
        handle: this.#requiredHandle(),
        idempotencyKey: this.#completeKey,
        operation: "complete",
        signal: this.#abort.signal,
        wholeChecksum: this.#whole.digestHex(),
      }),
      expectedRevision,
    );
    if (this.#disposed || this.#abort.signal.aborted || this.#state === "canceled") return;
    this.#completionUncertain = false;
    this.#completeKey = null;
    if (this.#applyTerminalResponse(response)) return;
    this.#revision = response.revision;
    this.#setState(response.state);
  }

  async #reconcile(): Promise<boolean> {
    const revision = this.#requiredRevision();
    const response = validateTransportResponse(
      "status",
      await this.#transport.send({
        grant: this.#requiredGrant(),
        handle: this.#requiredHandle(),
        operation: "status",
        signal: this.#abort.signal,
      }),
      revision,
    );
    if (this.#disposed || this.#abort.signal.aborted || this.#state === "canceled") return false;
    this.#mustReconcile = false;
    if (this.#applyTerminalResponse(response)) {
      this.#completionUncertain = false;
      this.#completeKey = null;
      return false;
    }
    this.#revision = response.revision;
    if (response.nextChunkIndex === undefined || response.nextChunkIndex > this.#offset) {
      throw new Error("upload_next_chunk_index_invalid");
    }
    this.#nextChunkIndex = response.nextChunkIndex;
    this.#setState(response.state);
    if (response.state === "verifying" || response.state === "ready") {
      this.#completionUncertain = false;
      this.#completeKey = null;
      return false;
    }
    return response.state === "queued" || response.state === "transferring";
  }

  async #hashRetainedPrefix(): Promise<void> {
    const file = this.#requiredFile();
    while (this.#hashedBytes < this.#offset) {
      const end = Math.min(this.#hashedBytes + this.#chunkBytes, this.#offset);
      const bytes = await file.slice(this.#hashedBytes, end).arrayBuffer();
      if (this.#inactive() || file !== this.#file) return;
      this.#whole.update(new Uint8Array(bytes));
      this.#hashedBytes = end;
    }
  }

  #applyTerminalResponse(response: UploadTransportResponse): boolean {
    if (
      response.state !== "canceled" &&
      response.state !== "expired" &&
      response.state !== "failed" &&
      response.state !== "finalized"
    ) {
      return false;
    }
    this.#revision = response.revision;
    this.#setState(response.state);
    this.#releaseAuthority();
    return true;
  }

  #requiredFile(): File {
    if (this.#file === null) throw new Error("upload_file_released");
    return this.#file;
  }

  #requiredHandle(): UploadHandle {
    if (this.#handle === null) throw new Error("upload_handle_missing");
    return this.#handle;
  }

  #requiredGrant(): SecretTransferGrant {
    if (this.#grant === null) throw new Error("upload_grant_missing");
    return this.#grant;
  }

  #requiredRevision(): string {
    if (this.#revision === null) throw new Error("upload_revision_missing");
    return this.#revision;
  }

  #nextKey(): string {
    const key = this.#randomness.idempotencyKey();
    validateUploadIdempotencyKey(key);
    return key;
  }

  #propose(handle: UploadHandle | null): void {
    if (this.#onHandle === undefined) this.#island.proposeUploadHandle(this.#field, handle);
    else this.#onHandle(handle);
  }

  #releaseAuthority(): void {
    if (this.#proposed || this.#handle !== null) {
      try {
        this.#propose(null);
      } catch {
        // Optional feature callbacks cannot retain current-document secrets.
      }
    }
    this.#proposed = false;
    this.#pending = null;
    this.#completeKey = null;
    this.#completionUncertain = false;
    this.#file = null;
    this.#grant = null;
    this.#handle = null;
    this.#changed();
  }

  #sendCancellation(
    handle: UploadHandle,
    grant: SecretTransferGrant,
    revision: string,
    idempotencyKey: string,
  ): void {
    this.#scheduleCleanup(
      Object.freeze({
        request: Object.freeze({
          expectedRevision: revision,
          grant,
          handle,
          idempotencyKey,
          operation: "cancel",
          signal: new AbortController().signal,
        }),
        transport: this.#transport,
      }),
    );
  }

  #setState(state: UploadPresentationState): void {
    if (this.#state === state) return;
    this.#state = state;
    this.#changed();
  }

  #canceled(): boolean {
    return this.#state === "canceled";
  }

  #inactive(): boolean {
    return this.#disposed || this.#abort.signal.aborted || this.#canceled();
  }

  #changed(): void {
    try {
      this.#onChange?.();
    } catch {
      // Presentation observation cannot change transfer ownership.
    }
  }
}

export const uploadSha256HexForTest = sha256Hex;
