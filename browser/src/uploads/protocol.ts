import {
  CanonicalError,
  canonicalize,
  parseCanonicalJson,
  type CanonicalLimits,
  type JsonObject,
  type JsonValue,
} from "../canonical.js";
import {
  validateUploadChecksum,
  validateUploadField,
  validateUploadHandle,
  validateUploadIdempotencyKey,
  validateUploadRevision,
  type UploadTransportRequest,
} from "./types.js";

const UPLOAD_LIMITS: CanonicalLimits = Object.freeze({
  maxBytes: 16_384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 4_096,
});
const MAX_U32 = 4_294_967_295;

export type UploadWireOperation =
  "cancel" | "complete" | "create" | "put_chunk" | "reacquire" | "status";

export class UploadProtocolError extends Error {
  constructor(
    readonly code:
      | "duplicate_field"
      | "invalid_field"
      | "noncanonical"
      | "unknown_field"
      | "unsupported_operation"
      | "unsupported_protocol",
  ) {
    super(code);
    this.name = "UploadProtocolError";
  }
}

function fail(code: UploadProtocolError["code"]): never {
  throw new UploadProtocolError(code);
}

function object(value: JsonValue): JsonObject {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("invalid_field");
  return value as JsonObject;
}

function operation(value: JsonValue | undefined): UploadWireOperation {
  switch (value) {
    case "cancel":
    case "complete":
    case "create":
    case "put_chunk":
    case "reacquire":
    case "status":
      return value;
    default:
      fail("unsupported_operation");
  }
}

function exact(fields: JsonObject, expected: readonly string[]): void {
  const present = Object.keys(fields);
  if (
    present.length !== expected.length ||
    expected.some((key) => !Object.prototype.hasOwnProperty.call(fields, key))
  ) {
    fail("unknown_field");
  }
}

function operationKeys(value: UploadWireOperation): readonly string[] {
  switch (value) {
    case "create":
      return ["expected_revision", "field", "idempotency_key", "operation", "protocol_version"];
    case "put_chunk":
      return [
        "checksum",
        "chunk_index",
        "expected_revision",
        "handle",
        "idempotency_key",
        "operation",
        "protocol_version",
        "size",
      ];
    case "complete":
      return [
        "expected_revision",
        "handle",
        "idempotency_key",
        "operation",
        "protocol_version",
        "whole_checksum",
      ];
    case "cancel":
      return ["expected_revision", "handle", "idempotency_key", "operation", "protocol_version"];
    case "reacquire":
    case "status":
      return ["handle", "operation", "protocol_version"];
  }
}

function validateFields(operationName: UploadWireOperation, fields: JsonObject): void {
  try {
    switch (operationName) {
      case "create":
        validateUploadRevision(fields["expected_revision"]);
        if (fields["expected_revision"] !== "0") fail("invalid_field");
        validateUploadField(fields["field"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        break;
      case "put_chunk":
        validateUploadHandle(fields["handle"]);
        validateUploadRevision(fields["expected_revision"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        if (
          typeof fields["chunk_index"] !== "number" ||
          !Number.isSafeInteger(fields["chunk_index"]) ||
          fields["chunk_index"] < 0 ||
          fields["chunk_index"] > MAX_U32 ||
          typeof fields["size"] !== "number" ||
          !Number.isSafeInteger(fields["size"]) ||
          fields["size"] < 1
        ) {
          fail("invalid_field");
        }
        validateUploadChecksum(fields["checksum"]);
        break;
      case "complete":
        validateUploadHandle(fields["handle"]);
        validateUploadRevision(fields["expected_revision"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        validateUploadChecksum(fields["whole_checksum"]);
        break;
      case "cancel":
        validateUploadHandle(fields["handle"]);
        validateUploadRevision(fields["expected_revision"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        break;
      case "reacquire":
      case "status":
        validateUploadHandle(fields["handle"]);
        break;
    }
  } catch (error: unknown) {
    if (error instanceof UploadProtocolError) throw error;
    fail("invalid_field");
  }
}

export function decodeUploadProtocolOperation(
  encoded: string,
): Readonly<{ operation: UploadWireOperation }> {
  let parsed: JsonValue;
  try {
    parsed = parseCanonicalJson(encoded, UPLOAD_LIMITS);
  } catch (error: unknown) {
    if (error instanceof CanonicalError && error.code === "duplicate_key") {
      fail("duplicate_field");
    }
    fail("invalid_field");
  }
  if (canonicalize(parsed) !== encoded) fail("noncanonical");
  const fields = object(parsed);
  if (fields["protocol_version"] !== 1) fail("unsupported_protocol");
  const operationName = operation(fields["operation"]);
  exact(fields, operationKeys(operationName));
  validateFields(operationName, fields);
  return Object.freeze({ operation: operationName });
}

export function validateUploadTransportRequest(request: UploadTransportRequest): void {
  try {
    switch (request.operation) {
      case "create":
        validateUploadField(request.field);
        validateUploadIdempotencyKey(request.idempotencyKey);
        break;
      case "put_chunk":
        validateUploadHandle(request.handle);
        validateUploadRevision(request.expectedRevision);
        validateUploadIdempotencyKey(request.idempotencyKey);
        validateUploadChecksum(request.checksum);
        if (
          !Number.isSafeInteger(request.chunkIndex) ||
          request.chunkIndex < 0 ||
          request.chunkIndex > MAX_U32 ||
          request.bytes.byteLength < 1
        ) {
          fail("invalid_field");
        }
        break;
      case "complete":
        validateUploadHandle(request.handle);
        validateUploadRevision(request.expectedRevision);
        validateUploadIdempotencyKey(request.idempotencyKey);
        validateUploadChecksum(request.wholeChecksum);
        break;
      case "cancel":
        validateUploadHandle(request.handle);
        validateUploadRevision(request.expectedRevision);
        validateUploadIdempotencyKey(request.idempotencyKey);
        break;
      case "status":
        validateUploadHandle(request.handle);
        break;
    }
  } catch (error: unknown) {
    if (error instanceof UploadProtocolError) throw error;
    fail("invalid_field");
  }
}
