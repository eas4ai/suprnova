import {
  canonicalize,
  parseCanonicalJson,
  type CanonicalLimits,
  type JsonObject,
  type JsonValue,
} from "../canonical.js";
import type {
  AsyncPayload,
  AsyncPayloadSchema,
  PresentationSignalSchema,
  AuthorizedLogicalSubscription,
  StreamPosition,
  ValidatedAsyncEnvelope,
} from "./types.js";

const MAX_U64 = (1n << 64n) - 1n;
const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;
const OPERATION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const SIGNAL_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const SIGNAL_SCOPE = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
const SUBSCRIPTION_ID = /^[A-Za-z0-9_-]{16,128}$/u;
const ASYNC_LIMITS: CanonicalLimits = Object.freeze({
  maxBytes: 64 * 1024,
  maxDepth: 8,
  maxEntries: 1_024,
  maxStringBytes: 4_096,
});
const MAX_PAYLOAD_BYTES = 32 * 1024;

export class AsyncEnvelopeError extends Error {
  constructor(readonly code: string) {
    super(code);
    this.name = "AsyncEnvelopeError";
  }
}

function fail(code: string): never {
  throw new AsyncEnvelopeError(code);
}

function record(value: JsonValue, code = "async_envelope_invalid"): JsonObject {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(code);
  return value as JsonObject;
}

function exact(value: JsonObject, keys: readonly string[], code = "async_envelope_invalid"): void {
  const present = Object.keys(value);
  if (
    present.length !== keys.length ||
    keys.some((key) => !Object.prototype.hasOwnProperty.call(value, key))
  ) {
    fail(code);
  }
}

function string(value: JsonValue | undefined, code = "async_envelope_invalid"): string {
  if (typeof value !== "string") fail(code);
  return value;
}

function integer(value: JsonValue | undefined, code = "async_envelope_invalid"): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) fail(code);
  return value;
}

function counter(value: JsonValue | undefined): bigint {
  const encoded = string(value);
  if (!/^(?:0|[1-9][0-9]*)$/u.test(encoded)) fail("async_position_invalid");
  const parsed = BigInt(encoded);
  if (parsed > MAX_U64) fail("async_position_invalid");
  return parsed;
}

function freezeJson(value: JsonValue): JsonValue {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) {
    const values = value as JsonValue[];
    return Object.freeze(values.map((item) => freezeJson(item)));
  }
  const source = value as JsonObject;
  const frozen = Object.create(null) as Record<string, JsonValue>;
  for (const key of Object.keys(source)) frozen[key] = freezeJson(source[key] ?? null);
  return Object.freeze(frozen);
}

function schemaMatches(schema: AsyncPayloadSchema, value: JsonValue): boolean {
  switch (schema) {
    case "json":
      return true;
    case "null":
      return value === null;
    case "boolean":
      return typeof value === "boolean";
    case "string":
      return typeof value === "string";
    case "i64":
      return typeof value === "number" && Number.isSafeInteger(value);
    case "u64":
      return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
    case "f64":
      return typeof value === "number" && Number.isFinite(value);
  }
}

function presentationSignalSchemaMatches(
  schema: PresentationSignalSchema,
  value: JsonValue,
): boolean {
  switch (schema) {
    case "null":
      return value === null;
    case "boolean":
      return typeof value === "boolean";
    case "string":
      return typeof value === "string";
    case "i64":
      return typeof value === "number" && Number.isSafeInteger(value);
    case "u64":
      return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
    default:
      return false;
  }
}

function targetValid(target: string): boolean {
  return (
    target === "self" ||
    target === "parent" ||
    target === "child" ||
    target === "document" ||
    /^named_island:[a-z][a-z0-9._-]{0,63}$/u.test(target) ||
    /^browser:[a-z][a-z0-9._-]{0,63}$/u.test(target)
  );
}

function payload(value: JsonValue, membership: AuthorizedLogicalSubscription): AsyncPayload {
  const fields = record(value, "async_payload_invalid");
  const kind = string(fields["kind"], "async_payload_invalid");
  if (new TextEncoder().encode(canonicalize(fields)).byteLength > MAX_PAYLOAD_BYTES) {
    fail("async_payload_too_large");
  }
  switch (kind) {
    case "refresh": {
      exact(fields, ["kind", "name"], "async_payload_invalid");
      if (fields["name"] !== "refresh") fail("async_payload_invalid");
      return Object.freeze({ kind: "refresh", name: "refresh" });
    }
    case "browser_event": {
      exact(
        fields,
        ["event", "kind", "payload", "schema_version", "target"],
        "async_payload_invalid",
      );
      const name = string(fields["event"], "async_payload_unregistered");
      const schemaVersion = integer(fields["schema_version"], "async_payload_unregistered");
      const target = string(fields["target"], "async_payload_unregistered");
      const event = membership.events.find((candidate) => candidate.name === name);
      const eventPayload = fields["payload"] ?? null;
      if (
        event === undefined ||
        !OPERATION_NAME.test(name) ||
        !Number.isSafeInteger(event.maximumFanout) ||
        event.maximumFanout < 1 ||
        event.maximumFanout > 256 ||
        event.version !== schemaVersion ||
        !targetValid(target) ||
        !event.targets.includes(target) ||
        !schemaMatches(event.schema, eventPayload)
      ) {
        fail("async_payload_unregistered");
      }
      return Object.freeze({
        event: name,
        kind: "browser_event",
        payload: freezeJson(eventPayload),
        schema_version: schemaVersion,
        target,
      });
    }
    case "presentation_signal": {
      exact(fields, ["kind", "name", "scope", "value"], "async_payload_invalid");
      const name = string(fields["name"], "async_payload_unregistered");
      const scope = string(fields["scope"], "async_payload_unregistered");
      const contract = membership.presentationSignals.find(
        (candidate) => candidate.name === name && candidate.scope === scope,
      );
      const signalValue = fields["value"] ?? null;
      if (
        contract === undefined ||
        !SIGNAL_NAME.test(name) ||
        !SIGNAL_SCOPE.test(scope) ||
        !presentationSignalSchemaMatches(contract.schema, signalValue)
      ) {
        fail("async_payload_unregistered");
      }
      return Object.freeze({
        kind: "presentation_signal",
        name,
        scope,
        value: freezeJson(signalValue),
      });
    }
    case "heartbeat":
      exact(fields, ["kind"], "async_payload_invalid");
      return Object.freeze({ kind: "heartbeat" });
    case "complete": {
      exact(fields, ["kind", "reason"], "async_payload_invalid");
      const reason = string(fields["reason"], "async_payload_invalid");
      if (
        reason !== "server_shutdown" &&
        reason !== "subscription_retired" &&
        reason !== "stream_completed"
      ) {
        fail("async_payload_invalid");
      }
      return Object.freeze({ kind: "complete", reason });
    }
    case "error": {
      exact(fields, ["code", "kind"], "async_payload_invalid");
      const code = string(fields["code"], "async_payload_invalid");
      if (
        code !== "authorization_lost" &&
        code !== "replay_unavailable" &&
        code !== "backpressure" &&
        code !== "stream_unavailable"
      ) {
        fail("async_payload_invalid");
      }
      return Object.freeze({ code, kind: "error" });
    }
    default:
      fail("async_payload_unsupported");
  }
}

export function decodeAsyncEnvelope(
  encoded: string,
  membership: AuthorizedLogicalSubscription,
): ValidatedAsyncEnvelope {
  let parsed: JsonValue;
  try {
    parsed = parseCanonicalJson(encoded, ASYNC_LIMITS);
  } catch {
    fail("async_envelope_invalid");
  }
  if (canonicalize(parsed) !== encoded) fail("async_envelope_noncanonical");
  const fields = record(parsed);
  exact(fields, ["payload", "position", "protocol_version", "stream", "subscription"]);
  if (fields["protocol_version"] !== 1) fail("async_protocol_unsupported");
  const subscriptionId = string(fields["subscription"]);
  if (!SUBSCRIPTION_ID.test(subscriptionId) || subscriptionId !== membership.subscriptionId) {
    fail("async_subscription_mismatch");
  }
  const stream = string(fields["stream"]);
  if (!OPERATION_NAME.test(stream) || stream !== membership.stream) fail("async_stream_mismatch");
  const positionFields = record(fields["position"] ?? null, "async_position_invalid");
  exact(positionFields, ["epoch", "sequence"], "async_position_invalid");
  const position: StreamPosition = Object.freeze({
    epoch: counter(positionFields["epoch"]),
    sequence: counter(positionFields["sequence"]),
  });
  return Object.freeze({
    payload: payload(fields["payload"] ?? null, membership),
    position,
    protocolVersion: 1,
    stream,
    subscriptionId,
  }) as ValidatedAsyncEnvelope;
}

export function inspectAsyncEnvelopeSubscription(encoded: string): string {
  let parsed: JsonValue;
  try {
    parsed = parseCanonicalJson(encoded, ASYNC_LIMITS);
  } catch {
    fail("async_envelope_invalid");
  }
  if (canonicalize(parsed) !== encoded) fail("async_envelope_noncanonical");
  const fields = record(parsed);
  exact(fields, ["payload", "position", "protocol_version", "stream", "subscription"]);
  if (fields["protocol_version"] !== 1) fail("async_protocol_unsupported");
  const subscriptionId = string(fields["subscription"]);
  if (!SUBSCRIPTION_ID.test(subscriptionId)) fail("async_subscription_mismatch");
  return subscriptionId;
}

export function comparePosition(left: StreamPosition, right: StreamPosition): number {
  if (left.epoch !== right.epoch) return left.epoch < right.epoch ? -1 : 1;
  if (left.sequence === right.sequence) return 0;
  return left.sequence < right.sequence ? -1 : 1;
}

export function isExactSuccessor(current: StreamPosition, candidate: StreamPosition): boolean {
  return (
    candidate.epoch === current.epoch &&
    current.sequence < MAX_U64 &&
    candidate.sequence === current.sequence + 1n
  );
}

export function validExpiration(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0 && value <= MAX_SAFE_INTEGER;
}
