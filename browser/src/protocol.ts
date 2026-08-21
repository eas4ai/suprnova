import { parseCanonicalJson } from "./canonical.js";
import { asArray, asNumber, asRecord, asString, requireExactKeys } from "./schema.js";

const encoder = new TextEncoder();
const MAX_U64 = 18_446_744_073_709_551_615n;

export class ProtocolValidationError extends Error {
  public constructor(public readonly code: string) {
    super(code);
    this.name = "ProtocolValidationError";
  }
}

export function validateUpdateRequest(text: string): void {
  try {
    validateUpdateRequestUnchecked(text);
  } catch (error: unknown) {
    throw normalizeProtocolError(error);
  }
}

function validateUpdateRequestUnchecked(text: string): void {
  const root = asRecord(parseCanonicalJson(text));
  requireExactKeys(root, [
    "base_revision",
    "component",
    "correlation_id",
    "extensions",
    "idempotency_key",
    "model_proposals",
    "operations",
    "protocol_version",
    "runtime_contract_version",
    "snapshot",
    "snapshot_schema_version",
  ]);
  if (
    asU16(root["protocol_version"]) !== 1 ||
    asU16(root["runtime_contract_version"]) !== 1 ||
    asU16(root["snapshot_schema_version"]) !== 1
  ) {
    throw new ProtocolValidationError("unsupported_protocol_version");
  }
  const baseRevision = decimalIdentity(root["base_revision"]);
  textIdentity(root["component"]);
  binaryIdentity(root["correlation_id"], 16, 32);
  binaryIdentity(root["idempotency_key"], 16, 32);
  validateExtensions(asRecord(root["extensions"]));
  const modelProposals = asRecord(root["model_proposals"]);
  if (Object.keys(modelProposals).length > 8) {
    throw new ProtocolValidationError("too_many_model_proposals");
  }
  for (const field of Object.keys(modelProposals)) textIdentity(field);
  const snapshot = asRecord(root["snapshot"]);
  const kind = asString(snapshot["kind"]);
  if (kind !== "instance" && kind !== "seed_promotion") {
    throw new ProtocolValidationError("invalid_snapshot_input_form");
  }
  if (kind === "instance") {
    requireSnapshotKeys(snapshot, ["envelope", "kind"]);
  } else {
    requireSnapshotKeys(snapshot, ["browser_nonce", "envelope", "kind"]);
    binaryIdentity(snapshot["browser_nonce"], 16, 32);
    if (baseRevision !== 0n) {
      throw new ProtocolValidationError("invalid_snapshot_input_form");
    }
  }
  asRecord(snapshot["envelope"]);
  const operations = asArray(root["operations"]);
  if (operations.length === 0 || operations.length > 8) {
    throw new ProtocolValidationError("too_many_operations");
  }
  let invoked = false;
  const synchronized = new Set<string>();
  for (const raw of operations) {
    const operation = asRecord(raw);
    const operationKind = asString(operation["kind"]);
    if (operationKind === "sync_model" && !invoked) {
      requireOperationKeys(operation, ["field", "kind"]);
      const field = textIdentity(operation["field"]);
      if (!(field in modelProposals) || synchronized.has(field)) {
        throw new ProtocolValidationError("incompatible_operation_batch");
      }
      synchronized.add(field);
    } else if (operationKind === "invoke_action" && !invoked) {
      requireOperationKeys(operation, ["arguments", "kind", "name"]);
      invoked = true;
      textIdentity(operation["name"]);
      const arguments_ = asRecord(operation["arguments"]);
      if (Object.keys(arguments_).length > 16) {
        throw new ProtocolValidationError("too_many_action_arguments");
      }
      for (const name of Object.keys(arguments_)) textIdentity(name);
    } else throw new ProtocolValidationError("incompatible_operation_batch");
  }
}

export function validateUpdateResponse(text: string): void {
  try {
    validateUpdateResponseUnchecked(text);
  } catch (error: unknown) {
    throw normalizeProtocolError(error);
  }
}

function validateUpdateResponseUnchecked(text: string): void {
  const root = asRecord(parseCanonicalJson(text));
  requireExactKeys(
    root,
    [
      "correlation_id",
      "effects",
      "events",
      "extensions",
      "outcome",
      "protocol_version",
      "validation",
    ],
    ["accepted_revision", "error", "redirect", "render", "snapshot"],
  );
  if (asU16(root["protocol_version"]) !== 1) {
    throw new ProtocolValidationError("unsupported_protocol_version");
  }
  binaryIdentity(root["correlation_id"], 16, 32);
  asArray(root["effects"]);
  asArray(root["events"]);
  asRecord(root["extensions"]);
  asRecord(root["validation"]);
  const outcome = asString(root["outcome"]);
  const redirect = root["redirect"];
  if (redirect !== undefined) {
    const target = asString(redirect);
    if (
      !target.startsWith("/") ||
      target.startsWith("//") ||
      target.includes("\\") ||
      encoder.encode(target).byteLength > 2_048 ||
      hasControlCharacter(target)
    ) {
      throw new ProtocolValidationError("unsafe_redirect");
    }
    if (root["snapshot"] !== undefined || root["render"] !== undefined) {
      throw new ProtocolValidationError("response_outcome_mismatch");
    }
  } else if (
    (outcome === "accepted" || outcome === "duplicate") &&
    root["snapshot"] === undefined
  ) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
}

function hasControlCharacter(value: string): boolean {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (
      codePoint !== undefined &&
      (codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f))
    ) {
      return true;
    }
  }
  return false;
}

function asU16(value: unknown): number {
  const number = asNumber(value);
  if (!Number.isInteger(number) || number < 0 || number > 65_535) {
    throw new TypeError("invalid_u16");
  }
  return number;
}

function decimalIdentity(value: unknown): bigint {
  const text = asString(value);
  if (!/^(?:0|[1-9][0-9]*)$/u.test(text)) identityError();
  const parsed = BigInt(text);
  if (parsed > MAX_U64) identityError();
  return parsed;
}

function textIdentity(value: unknown): string {
  const text = asString(value);
  if (text.length === 0 || text.length > 128 || !/^[A-Za-z0-9._:/-]+$/u.test(text)) {
    identityError();
  }
  return text;
}

function binaryIdentity(value: unknown, minimum: number, maximum: number): void {
  const text = asString(value);
  try {
    if (!/^[A-Za-z0-9_-]+$/u.test(text) || text.includes("=")) identityError();
    const base64 = text.replace(/-/gu, "+").replace(/_/gu, "/");
    const binary = atob(base64.padEnd(Math.ceil(base64.length / 4) * 4, "="));
    const canonical = btoa(binary).replace(/\+/gu, "-").replace(/\//gu, "_").replace(/=+$/u, "");
    if (binary.length < minimum || binary.length > maximum || canonical !== text) identityError();
  } catch (error: unknown) {
    if (error instanceof ProtocolValidationError) throw error;
    identityError();
  }
}

function identityError(): never {
  throw new ProtocolValidationError("invalid_protocol_identity");
}

function requireSnapshotKeys(
  value: Readonly<Record<string, unknown>>,
  expected: readonly string[],
): void {
  try {
    requireExactKeys(value, expected);
  } catch {
    throw new ProtocolValidationError("invalid_snapshot_input_form");
  }
}

function requireOperationKeys(
  value: Readonly<Record<string, unknown>>,
  expected: readonly string[],
): void {
  try {
    requireExactKeys(value, expected);
  } catch {
    throw new ProtocolValidationError("ambiguous_operation");
  }
}

function validateExtensions(value: Readonly<Record<string, unknown>>): void {
  const names = Object.keys(value);
  if (
    names.length > 8 ||
    names.some(
      (name) => !name.startsWith("x_") || name.length > 64 || !/^[A-Za-z0-9_.-]+$/u.test(name),
    )
  ) {
    throw new ProtocolValidationError("invalid_protocol_extension");
  }
}

function normalizeProtocolError(error: unknown): ProtocolValidationError {
  if (error instanceof ProtocolValidationError) return error;
  const code = error instanceof Error ? error.message : "invalid_protocol_envelope";
  const mapped = new Map<string, string>([
    ["input_too_large", "protocol_input_too_large"],
    ["input_too_deep", "protocol_input_too_deep"],
    ["too_many_entries", "protocol_too_many_entries"],
    ["duplicate_key", "protocol_duplicate_field"],
  ]).get(code);
  return new ProtocolValidationError(mapped ?? "invalid_protocol_envelope");
}
