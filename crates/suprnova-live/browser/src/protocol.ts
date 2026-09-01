import { parseCanonicalJson, type JsonObject, type JsonValue } from "./canonical.js";
import type {
  ErrorCategory,
  RecoveryInstruction,
  ResponseOutcome,
  ValidatedChildDelivery,
  ValidatedEmission,
  ValidatedLiveError,
  ValidatedRender,
  ValidatedResponse,
} from "./application/types.js";
import { inspectSnapshotPublicView } from "./protocol/snapshot-view.js";
import {
  asArray,
  asJsonValue,
  asNullableRecord,
  asNumber,
  asRecord,
  asString,
  requireExactKeys,
} from "./schema.js";

const MAX_U64 = 18_446_744_073_709_551_615n;

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

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
  const version = asU16(root["protocol_version"]);
  if (version === 1) validateUpdateRequestV1(root);
  else if (version === 2) validateUpdateRequestV2(root);
  else throw new ProtocolValidationError("unsupported_protocol_version");
}

function validateUpdateRequestV1(root: Readonly<Record<string, unknown>>): void {
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

export function parseUpdateResponse(text: string): ValidatedResponse {
  try {
    return parseUpdateResponseUnchecked(text);
  } catch (error: unknown) {
    throw normalizeProtocolError(error);
  }
}

function parseUpdateResponseUnchecked(text: string): ValidatedResponse {
  const root = asRecord(parseCanonicalJson(text));
  const version = asU16(root["protocol_version"]);
  if (version === 1) {
    validateUpdateResponseV1(root);
    return materializeResponse(root, 1);
  }
  if (version === 2) {
    validateUpdateResponseV2(root);
    return materializeResponse(root, 2);
  }
  throw new ProtocolValidationError("unsupported_protocol_version");
}

function validateUpdateResponseUnchecked(text: string): void {
  const root = asRecord(parseCanonicalJson(text));
  const version = asU16(root["protocol_version"]);
  if (version === 1) validateUpdateResponseV1(root);
  else if (version === 2) validateUpdateResponseV2(root);
  else throw new ProtocolValidationError("unsupported_protocol_version");
}

function validateUpdateResponseV1(root: Readonly<Record<string, unknown>>): void {
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
  const effects = validateEmissions(root["effects"], 8);
  const events = validateEmissions(root["events"], 8);
  validateExtensions(asRecord(root["extensions"]));
  const validation = asRecord(root["validation"]);
  if (Object.keys(validation).length > 16) {
    throw new ProtocolValidationError("protocol_too_many_entries");
  }
  const outcome = asString(root["outcome"]);
  if (!["accepted", "duplicate", "rejected", "refresh_required", "fatal"].includes(outcome)) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  const redirect = root["redirect"];
  if (redirect !== undefined) {
    const target = asString(redirect);
    if (
      !target.startsWith("/") ||
      target.startsWith("//") ||
      target.includes("\\") ||
      utf8Length(target) > 2_048 ||
      hasControlCharacter(target)
    ) {
      throw new ProtocolValidationError("unsafe_redirect");
    }
    if (root["snapshot"] !== undefined || root["render"] !== undefined) {
      throw new ProtocolValidationError("response_outcome_mismatch");
    }
  }
  const acceptedRevision = root["accepted_revision"];
  const snapshot = root["snapshot"];
  const render = root["render"];
  if (acceptedRevision !== undefined) decimalIdentity(acceptedRevision);
  if (snapshot !== undefined) asRecord(snapshot);
  if (render !== undefined) validateRender(render);
  const committed =
    acceptedRevision !== undefined &&
    snapshot !== undefined &&
    render !== undefined &&
    redirect === undefined;
  const statePresent =
    acceptedRevision !== undefined || snapshot !== undefined || render !== undefined;
  const terminal =
    redirect !== undefined &&
    !statePresent &&
    Object.keys(validation).length === 0 &&
    events.length === 0 &&
    effects.length === 0;
  const accepted = outcome === "accepted" || outcome === "duplicate";
  const recovery = validateOptionalLiveError(root["error"]);
  if (accepted && (recovery !== undefined || (!committed && !terminal))) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  if (
    !accepted &&
    (statePresent ||
      terminal ||
      events.length !== 0 ||
      effects.length !== 0 ||
      recovery === undefined)
  ) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  if (!accepted) validateRecovery(outcome, recovery, validation);
}

function validateUpdateRequestV2(root: Readonly<Record<string, unknown>>): void {
  requireExactKeys(root, [
    "base_revision",
    "child_parameters",
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
    asU16(root["protocol_version"]) !== 2 ||
    asU16(root["runtime_contract_version"]) !== 2 ||
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
  validateSnapshot(root["snapshot"], baseRevision);

  const childParameters = asNullableRecord(root["child_parameters"]);
  const hasChildParameters = childParameters !== undefined;

  const operations = asArray(root["operations"]);
  if (operations.length === 0 || operations.length > 8) {
    throw new ProtocolValidationError("too_many_operations");
  }
  let invoked = false;
  let lifecycle: "params_changed" | "lazy_complete" | "fresh_render" | undefined;
  const synchronized = new Set<string>();
  for (const raw of operations) {
    const operation = asRecord(raw);
    const kind = asString(operation["kind"]);
    if (kind === "params_changed" || kind === "lazy_complete" || kind === "fresh_render") {
      requireOperationKeys(operation, ["kind"]);
      lifecycle = kind;
      continue;
    }
    if (kind === "sync_model" && !invoked) {
      requireOperationKeys(operation, ["field", "kind"]);
      const field = textIdentity(operation["field"]);
      if (!(field in modelProposals) || synchronized.has(field)) {
        throw new ProtocolValidationError("incompatible_operation_batch");
      }
      synchronized.add(field);
    } else if (kind === "invoke_action" && !invoked) {
      requireOperationKeys(operation, ["arguments", "kind", "name"]);
      invoked = true;
      textIdentity(operation["name"]);
      const arguments_ = asRecord(operation["arguments"]);
      if (Object.keys(arguments_).length > 16) {
        throw new ProtocolValidationError("too_many_action_arguments");
      }
      for (const name of Object.keys(arguments_)) textIdentity(name);
    } else if (kind === "sync_model" || kind === "invoke_action") {
      throw new ProtocolValidationError("incompatible_operation_batch");
    } else throw new ProtocolValidationError("ambiguous_operation");
  }
  if (lifecycle !== undefined) {
    const childMatch = lifecycle === "params_changed" ? hasChildParameters : !hasChildParameters;
    if (operations.length !== 1 || Object.keys(modelProposals).length !== 0 || !childMatch) {
      throw new ProtocolValidationError("incompatible_operation_batch");
    }
  } else if (hasChildParameters) {
    throw new ProtocolValidationError("incompatible_operation_batch");
  }
}

function validateSnapshot(value: unknown, baseRevision: bigint): void {
  const snapshot = asRecord(value);
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
}

function validateUpdateResponseV2(root: Readonly<Record<string, unknown>>): void {
  requireExactKeys(
    root,
    [
      "child_deliveries",
      "correlation_id",
      "effects",
      "events",
      "extensions",
      "outcome",
      "protocol_version",
      "url_intent",
      "validation",
    ],
    ["accepted_revision", "error", "redirect", "render", "snapshot"],
  );
  if (asU16(root["protocol_version"]) !== 2) {
    throw new ProtocolValidationError("unsupported_protocol_version");
  }
  binaryIdentity(root["correlation_id"], 16, 32);
  const effects = validateEmissions(root["effects"], 8);
  const events = validateEmissions(root["events"], 8);
  validateExtensions(asRecord(root["extensions"]));
  const validation = asRecord(root["validation"]);
  if (Object.keys(validation).length > 16) {
    throw new ProtocolValidationError("protocol_too_many_entries");
  }
  const outcome = asString(root["outcome"]);
  if (!["accepted", "duplicate", "rejected", "refresh_required", "fatal"].includes(outcome)) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }

  const childDeliveries = asArray(root["child_deliveries"]);
  if (childDeliveries.length > 8) {
    throw new ProtocolValidationError("protocol_too_many_entries");
  }
  for (const raw of childDeliveries) {
    const delivery = asRecord(raw);
    requireExactKeys(delivery, ["child_instance", "envelope", "parameter_hash"]);
    binaryIdentity(delivery["child_instance"], 16, 32);
    binaryIdentity(delivery["parameter_hash"], 32, 32);
    asRecord(delivery["envelope"]);
  }

  let reflected = false;
  let navigated = false;
  const urlIntent = asNullableRecord(root["url_intent"]);
  if (urlIntent !== undefined) {
    const intent = urlIntent;
    requireExactKeys(intent, ["kind", "target"]);
    const kind = asString(intent["kind"]);
    validateSafeTarget(intent["target"]);
    if (kind === "reflected") reflected = true;
    else if (kind === "navigated") navigated = true;
    else throw new ProtocolValidationError("invalid_protocol_envelope");
  }

  const redirect = root["redirect"];
  if (redirect !== undefined) validateSafeTarget(redirect);
  if (redirect !== undefined && urlIntent !== undefined) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  const acceptedRevision = root["accepted_revision"];
  const snapshot = root["snapshot"];
  const render = root["render"];
  if (acceptedRevision !== undefined) decimalIdentity(acceptedRevision);
  if (snapshot !== undefined) asRecord(snapshot);
  if (render !== undefined) validateRender(render);
  const committed =
    acceptedRevision !== undefined &&
    snapshot !== undefined &&
    render !== undefined &&
    redirect === undefined;
  const statePresent =
    acceptedRevision !== undefined || snapshot !== undefined || render !== undefined;
  const terminal =
    (redirect !== undefined || navigated) &&
    root["accepted_revision"] === undefined &&
    root["snapshot"] === undefined &&
    root["render"] === undefined &&
    Object.keys(validation).length === 0 &&
    events.length === 0 &&
    effects.length === 0;
  const accepted = outcome === "accepted" || outcome === "duplicate";
  const recovery = validateOptionalLiveError(root["error"]);
  if (accepted && (recovery !== undefined || (!committed && !terminal))) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  if (
    !accepted &&
    (statePresent ||
      terminal ||
      childDeliveries.length !== 0 ||
      urlIntent !== undefined ||
      events.length !== 0 ||
      effects.length !== 0 ||
      recovery === undefined)
  ) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  if (!accepted) validateRecovery(outcome, recovery, validation);
  if ((reflected || childDeliveries.length !== 0) && !committed) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
  if (navigated && (committed || childDeliveries.length !== 0)) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
}

function materializeResponse(
  root: Readonly<Record<string, unknown>>,
  protocol: 1 | 2,
): ValidatedResponse {
  const outcome = asString(root["outcome"]) as ResponseOutcome;
  const correlationId = asString(root["correlation_id"]);
  const effects = materializeEmissions(root["effects"]);
  const events = materializeEmissions(root["events"]);
  const extensions = frozenObject(root["extensions"]);
  const validation = frozenObject(root["validation"]);
  const redirect = root["redirect"];
  const urlIntent = protocol === 2 ? asNullableRecord(root["url_intent"]) : undefined;
  const navigated = urlIntent?.["kind"] === "navigated";
  if (redirect !== undefined || navigated) {
    return Object.freeze({
      correlationId,
      effects,
      events,
      extensions,
      kind: "navigation",
      navigation: redirect !== undefined ? "redirect" : "navigated",
      outcome,
      protocol,
      target: asString(redirect ?? urlIntent?.["target"]),
      validation,
    });
  }

  if (outcome === "accepted" || outcome === "duplicate") {
    const snapshot = frozenObject(root["snapshot"]);
    const snapshotView = inspectSnapshotPublicView(snapshot);
    const childDeliveries =
      protocol === 2 ? materializeChildDeliveries(root["child_deliveries"]) : Object.freeze([]);
    const reflectedUrl = urlIntent?.["kind"] === "reflected" ? asString(urlIntent["target"]) : null;
    return Object.freeze({
      acceptedRevision: decimalIdentity(root["accepted_revision"]),
      childDeliveries,
      correlationId,
      effects,
      events,
      extensions,
      kind: "committed",
      outcome,
      protocol,
      reflectedUrl,
      render: materializeRender(root["render"]),
      snapshot,
      snapshotView,
      validation,
    });
  }

  const error = materializeLiveError(root["error"]);
  const failureBase = {
    correlationId,
    effects,
    error,
    events,
    extensions,
    protocol,
    validation,
  } as const;
  if (outcome === "refresh_required") {
    return Object.freeze({
      ...failureBase,
      kind: "recovery",
      outcome,
      recovery: error.recovery as "refresh_island" | "remount_island" | "navigate",
    });
  }
  if (outcome === "fatal") {
    return Object.freeze({
      ...failureBase,
      kind: "fatal",
      outcome,
      recovery: error.recovery as "stop" | "navigate",
    });
  }
  return Object.freeze({
    ...failureBase,
    kind: "rejected",
    outcome,
    recovery: error.recovery as "retain_dom" | "retry",
  });
}

function materializeEmissions(value: unknown): readonly ValidatedEmission[] {
  return Object.freeze(
    asArray(value).map((raw) => {
      const emission = asRecord(raw);
      return Object.freeze({
        name: asString(emission["name"]),
        payload: freezeJson(asJsonValue(emission["payload"])),
      });
    }),
  );
}

function materializeChildDeliveries(value: unknown): readonly ValidatedChildDelivery[] {
  return Object.freeze(
    asArray(value).map((raw) => {
      const delivery = asRecord(raw);
      return Object.freeze({
        childInstance: asString(delivery["child_instance"]),
        envelope: frozenObject(delivery["envelope"]),
        parameterHash: asString(delivery["parameter_hash"]),
      });
    }),
  );
}

function materializeRender(value: unknown): ValidatedRender {
  const render = asRecord(value);
  return asString(render["kind"]) === "html"
    ? Object.freeze({ html: asString(render["html"]), kind: "html" })
    : Object.freeze({ kind: "no_render" });
}

function materializeLiveError(value: unknown): ValidatedLiveError {
  const error = asRecord(value);
  return Object.freeze({
    category: asString(error["category"]) as ErrorCategory,
    detail: asString(error["detail"]),
    recovery: asString(error["recovery"]) as RecoveryInstruction,
  });
}

function frozenObject(value: unknown): JsonObject {
  const frozen = freezeJson(asJsonValue(value));
  if (frozen === null || typeof frozen !== "object" || Array.isArray(frozen)) {
    throw new TypeError("expected_object");
  }
  return frozen as JsonObject;
}

function freezeJson(value: JsonValue): JsonValue {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return Object.freeze(value.map(freezeJson));
  const result = Object.create(null) as Record<string, JsonValue>;
  for (const [key, item] of Object.entries(value)) result[key] = freezeJson(item);
  return Object.freeze(result);
}

function validateEmissions(value: unknown, maximum: number): readonly unknown[] {
  const emissions = asArray(value);
  if (emissions.length > maximum) {
    throw new ProtocolValidationError("protocol_too_many_entries");
  }
  for (const raw of emissions) {
    const emission = asRecord(raw);
    requireExactKeys(emission, ["name", "payload"]);
    textIdentity(emission["name"]);
  }
  return emissions;
}

function validateRender(value: unknown): void {
  const render = asRecord(value);
  const kind = asString(render["kind"]);
  if (kind === "html") {
    requireExactKeys(render, ["html", "kind"]);
    if (utf8Length(asString(render["html"])) > 32 * 1_024) {
      throw new ProtocolValidationError("protocol_input_too_large");
    }
  } else if (kind === "no_render") requireExactKeys(render, ["kind"]);
  else throw new ProtocolValidationError("response_outcome_mismatch");
}

function validateOptionalLiveError(value: unknown): string | undefined {
  if (value === undefined) return undefined;
  const error = asRecord(value);
  requireExactKeys(error, ["category", "detail", "recovery"]);
  const category = asString(error["category"]);
  const categories = [
    "protocol",
    "validation",
    "authentication",
    "authorization",
    "csrf",
    "snapshot",
    "revision",
    "render",
    "morph",
    "provider",
    "cache",
    "upload",
    "compatibility",
    "size_limit",
    "rate_limit",
    "security",
    "internal",
  ];
  if (!categories.includes(category)) {
    throw new ProtocolValidationError("invalid_protocol_envelope");
  }
  textIdentity(error["detail"]);
  const recovery = asString(error["recovery"]);
  if (
    !["retain_dom", "retry", "refresh_island", "remount_island", "navigate", "stop"].includes(
      recovery,
    )
  ) {
    throw new ProtocolValidationError("invalid_protocol_envelope");
  }
  return recovery;
}

function validateRecovery(
  outcome: string,
  recovery: string | undefined,
  validation: Readonly<Record<string, unknown>>,
): void {
  const allowed =
    outcome === "rejected"
      ? ["retain_dom", "retry"]
      : outcome === "refresh_required"
        ? ["refresh_island", "remount_island", "navigate"]
        : ["stop", "navigate"];
  if (recovery === undefined || !allowed.includes(recovery)) {
    throw new ProtocolValidationError("error_recovery_mismatch");
  }
  if (outcome !== "rejected" && Object.keys(validation).length !== 0) {
    throw new ProtocolValidationError("response_outcome_mismatch");
  }
}

function validateSafeTarget(value: unknown): void {
  const target = asString(value);
  const path = target.split(/[?#]/u, 1)[0] ?? "";
  if (
    !target.startsWith("/") ||
    target.startsWith("//") ||
    target.includes("\\") ||
    utf8Length(target) > 2_048 ||
    hasControlCharacter(target) ||
    !hasNormalizedPathSegments(path)
  ) {
    throw new ProtocolValidationError("unsafe_redirect");
  }
}

function hasNormalizedPathSegments(path: string): boolean {
  return path.split("/").every((segment) => {
    const decoded: number[] = [];
    for (let index = 0; index < segment.length; index += 1) {
      const character = segment[index];
      if (character !== "%") {
        decoded.push(character?.charCodeAt(0) ?? 0);
        continue;
      }
      const encoded = segment.slice(index + 1, index + 3);
      if (!/^[0-9A-Fa-f]{2}$/u.test(encoded)) return false;
      const byte = Number.parseInt(encoded, 16);
      if (byte === 0x2f || byte === 0x5c || byte <= 0x1f || byte === 0x7f) return false;
      decoded.push(byte);
      index += 2;
    }
    return (
      !(decoded.length === 1 && decoded[0] === 0x2e) &&
      !(decoded.length === 2 && decoded[0] === 0x2e && decoded[1] === 0x2e)
    );
  });
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
