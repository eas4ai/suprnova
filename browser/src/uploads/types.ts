import type { IslandExtensionIdentity } from "../extensions/registry.js";
import { parseUploadProtocolState } from "./state.js";

export const DEFAULT_UPLOAD_CHUNK_BYTES = 256 * 1024;
export const MAX_UPLOAD_FILES_PER_DOCUMENT = 64;
export const MAX_UPLOAD_HANDLE_COUNT = 64;
export const MAX_UPLOAD_ACTIVE_TRANSFERS = 16;
export const MAX_UPLOAD_CHUNK_BYTES = 4 * 1024 * 1024;
export const MAX_UPLOAD_QUEUE_BYTES = 4 * 1024 * 1024;

const UPLOAD_FIELD = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/u;
const UPLOAD_HANDLE = /^[0-9a-f]{8}-[0-9a-f]{4}-[47][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
const IDEMPOTENCY_KEY = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
const SHA256 = /^[0-9a-f]{64}$/u;
const REVISION = /^(?:0|[1-9][0-9]{0,19})$/u;
const MAX_U64 = 18_446_744_073_709_551_615n;
const MAX_FILE_NAME_UNITS = 255;
const MAX_MIME_UNITS = 255;
const MAX_GRANT_UNITS = 4096;

export type UploadHandle = string;
export type SecretTransferGrant = string;
export type UploadHandleProposal = UploadHandle | readonly UploadHandle[] | null;
export type UploadHandleProposalDisposition = "accepted" | "unchanged" | "retired";

export type UploadPresentationState =
  | "queued"
  | "transferring"
  | "verifying"
  | "ready"
  | "finalizing"
  | "finalized"
  | "interrupted"
  | "failed"
  | "canceled"
  | "expired";

export interface UploadFileIdentity {
  readonly lastModified: number;
  readonly name: string;
  readonly size: number;
  readonly type: string;
}

export interface UploadIslandPort {
  readonly element: Element;
  readonly identity: IslandExtensionIdentity;
  proposeUploadHandle(
    field: string,
    proposal: UploadHandleProposal,
  ): UploadHandleProposalDisposition;
}

export interface UploadSelection {
  readonly field: string;
  readonly input: HTMLInputElement;
  readonly island: UploadIslandPort;
}

export interface UploadConnectivity {
  online(): boolean;
}

export interface UploadRandomness {
  idempotencyKey(): string;
}

interface UploadRequestBase {
  readonly signal: AbortSignal;
}

export interface CreateUploadRequest extends UploadRequestBase {
  readonly operation: "create";
  readonly field: string;
  readonly file: UploadFileIdentity;
  readonly idempotencyKey: string;
  readonly island: IslandExtensionIdentity;
}

export interface PutUploadChunkRequest extends UploadRequestBase {
  readonly operation: "put_chunk";
  readonly bytes: ArrayBuffer;
  readonly checksum: string;
  readonly chunkIndex: number;
  readonly expectedRevision: string;
  readonly grant: SecretTransferGrant;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
}

export interface CompleteUploadRequest extends UploadRequestBase {
  readonly operation: "complete";
  readonly expectedRevision: string;
  readonly grant: SecretTransferGrant;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
  readonly wholeChecksum: string;
}

export interface CancelUploadRequest extends UploadRequestBase {
  readonly operation: "cancel";
  readonly expectedRevision: string;
  readonly grant: SecretTransferGrant;
  readonly handle: UploadHandle;
  readonly idempotencyKey: string;
}

export interface StatusUploadRequest extends UploadRequestBase {
  readonly operation: "status";
  readonly grant: SecretTransferGrant;
  readonly handle: UploadHandle;
}

export type UploadTransportRequest =
  | CreateUploadRequest
  | PutUploadChunkRequest
  | CompleteUploadRequest
  | CancelUploadRequest
  | StatusUploadRequest;

export interface UploadTransportResponse {
  readonly grant?: SecretTransferGrant;
  readonly handle?: UploadHandle;
  readonly nextChunkIndex?: number;
  readonly revision: string;
  readonly state: UploadPresentationState;
}

export interface UploadTransport {
  send(request: UploadTransportRequest): Promise<UploadTransportResponse>;
}

export interface ReacquiredUpload {
  readonly fileIdentity: UploadFileIdentity;
  readonly grant: SecretTransferGrant;
  readonly nextChunkIndex: number;
  readonly revision: string;
  readonly state: "queued" | "transferring" | "verifying";
  readonly uploadedBytes: number;
}

export interface UploadApplicationPort {
  reacquire(
    request: Readonly<{
      field: string;
      fileIdentity: UploadFileIdentity;
      handle: UploadHandle;
    }>,
  ): Promise<ReacquiredUpload>;
}

export interface ReacquiredTransfer extends ReacquiredUpload {
  readonly file: File;
  readonly handle: UploadHandle;
}

export interface UploadTransferSnapshot {
  readonly field: string;
  readonly handle: UploadHandle | null;
  readonly name: string;
  readonly retainedChunks: number;
  readonly revision: string | null;
  readonly sentBytes: number;
  readonly size: number;
  readonly state: UploadPresentationState;
}

export interface UploadSecretSnapshot {
  readonly chunks: number;
  readonly files: number;
  readonly grants: number;
}

export interface UploadManagerSnapshot {
  readonly cleanupObligations: number;
  readonly uploads: readonly UploadTransferSnapshot[];
}

export interface UploadManagerOptions {
  readonly application?: UploadApplicationPort | undefined;
  readonly chunkBytes: number;
  readonly connectivity: UploadConnectivity;
  readonly maxActive: number;
  readonly maxItems: number;
  readonly maxQueueBytes: number;
  readonly randomness: UploadRandomness;
  readonly transport: UploadTransport;
}

const RESPONSE_STATES = Object.freeze({
  cancel: ["canceled", "expired", "finalized"] as const,
  complete: ["verifying", "ready", "failed", "canceled", "expired"] as const,
  create: ["queued"] as const,
  put_chunk: ["transferring", "verifying", "ready", "failed", "canceled", "expired"] as const,
  status: [
    "queued",
    "transferring",
    "verifying",
    "ready",
    "finalizing",
    "finalized",
    "failed",
    "canceled",
    "expired",
  ] as const,
}) satisfies Readonly<
  Record<UploadTransportRequest["operation"], readonly UploadPresentationState[]>
>;

export function validateUploadField(field: unknown): asserts field is string {
  if (typeof field !== "string" || !UPLOAD_FIELD.test(field)) {
    throw new Error("upload_field_invalid");
  }
}

export function validateUploadHandle(handle: unknown): asserts handle is UploadHandle {
  if (typeof handle !== "string" || !UPLOAD_HANDLE.test(handle)) {
    throw new Error("upload_handle_invalid");
  }
}

export function validateUploadProposal(
  proposal: unknown,
): asserts proposal is UploadHandleProposal {
  if (proposal === null) return;
  if (typeof proposal === "string") {
    validateUploadHandle(proposal);
    return;
  }
  if (
    !Array.isArray(proposal) ||
    proposal.length < 1 ||
    proposal.length > MAX_UPLOAD_HANDLE_COUNT
  ) {
    throw new Error("upload_handle_proposal_invalid");
  }
  const handles = new Set<string>();
  for (const handle of proposal) {
    validateUploadHandle(handle);
    if (handles.has(handle)) throw new Error("upload_handle_proposal_invalid");
    handles.add(handle);
  }
}

export function validateUploadIdempotencyKey(value: unknown): asserts value is string {
  if (typeof value !== "string" || !IDEMPOTENCY_KEY.test(value)) {
    throw new Error("upload_idempotency_key_invalid");
  }
}

export function validateUploadChecksum(value: unknown): asserts value is string {
  if (typeof value !== "string" || !SHA256.test(value)) {
    throw new Error("upload_checksum_invalid");
  }
}

export function validateUploadRevision(value: unknown): asserts value is string {
  if (typeof value !== "string" || !REVISION.test(value) || BigInt(value) > MAX_U64) {
    throw new Error("upload_revision_invalid");
  }
}

export function validateUploadedBytes(value: unknown, fileSize: number): asserts value is number {
  if (!Number.isSafeInteger(value) || typeof value !== "number" || value < 0 || value > fileSize) {
    throw new Error("upload_reacquire_offset_invalid");
  }
}

export function validateNextChunkIndex(value: unknown): asserts value is number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new Error("upload_next_chunk_index_invalid");
  }
}

export function validateTransferGrant(value: unknown): asserts value is SecretTransferGrant {
  if (
    typeof value !== "string" ||
    value.length < 1 ||
    value.length > MAX_GRANT_UNITS ||
    hasControlCharacter(value)
  ) {
    throw new Error("upload_transfer_grant_invalid");
  }
}

export function uploadFileIdentity(file: File): UploadFileIdentity {
  const segments = file.name.replace(/\\/gu, "/").split("/");
  const leaf = segments[segments.length - 1] ?? "";
  const name = Array.from(leaf)
    .filter((character) => {
      const code = character.codePointAt(0) ?? 0;
      return code > 31 && code !== 127;
    })
    .join("")
    .slice(0, MAX_FILE_NAME_UNITS);
  const type = file.type.trim().toLowerCase().slice(0, MAX_MIME_UNITS);
  if (
    name.length === 0 ||
    !Number.isSafeInteger(file.size) ||
    file.size < 0 ||
    !Number.isSafeInteger(file.lastModified) ||
    file.lastModified < 0
  ) {
    throw new Error("upload_file_identity_invalid");
  }
  return Object.freeze({ lastModified: file.lastModified, name, size: file.size, type });
}

export function uploadFileIdentitiesEqual(
  left: UploadFileIdentity,
  right: UploadFileIdentity,
): boolean {
  return (
    left.lastModified === right.lastModified &&
    left.name === right.name &&
    left.size === right.size &&
    left.type === right.type
  );
}

export function validateTransportResponse(
  operation: UploadTransportRequest["operation"],
  response: UploadTransportResponse,
  expectedRevision?: string,
): UploadTransportResponse {
  const candidate: unknown = response;
  if ((typeof candidate !== "object" && typeof candidate !== "function") || candidate === null) {
    throw new Error("upload_transport_response_invalid");
  }
  validateUploadRevision(response.revision);
  if (response.state !== "interrupted") parseUploadProtocolState(response.state);
  const allowed: readonly UploadPresentationState[] = RESPONSE_STATES[operation];
  if (!allowed.includes(response.state)) {
    throw new Error("upload_transport_response_invalid");
  }
  if (expectedRevision !== undefined) {
    validateUploadRevision(expectedRevision);
    const received = BigInt(response.revision);
    const expected = BigInt(expectedRevision);
    if (received < expected || (operation !== "status" && received === expected)) {
      throw new Error("upload_transport_revision_invalid");
    }
  }
  if (operation === "create") {
    validateUploadHandle(response.handle);
    validateTransferGrant(response.grant);
  } else if (response.handle !== undefined || response.grant !== undefined) {
    throw new Error("upload_transport_response_invalid");
  }
  if (operation === "status") validateNextChunkIndex(response.nextChunkIndex);
  else if (response.nextChunkIndex !== undefined)
    throw new Error("upload_transport_response_invalid");
  return Object.freeze({ ...response });
}

function hasControlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}
