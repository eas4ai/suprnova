import { readFile } from "node:fs/promises";

import { AsyncEnvelopeError, decodeAsyncEnvelope } from "../../src/async-updates/envelope.js";
import { ContinuityMachine } from "../../src/async-updates/continuity.js";
import type {
  AuthorizedLogicalSubscription,
  StreamPosition,
} from "../../src/async-updates/types.js";
import {
  CanonicalError,
  canonicalize,
  parseCanonicalJson,
  type JsonObject,
  type JsonValue,
} from "../../src/canonical.js";

const FIXTURE_ROOT = new URL("../../../fixtures/v4/", import.meta.url);
const SUBSCRIPTION = "c3Vic2NyaXB0aW9uLTAwMQ";
const UPLOAD_LIMITS = Object.freeze({
  maxBytes: 16_384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 4_096,
});

type UploadWireOperation = "cancel" | "complete" | "create" | "put_chunk" | "reacquire" | "status";

type InternalUploadTransition =
  | "accept"
  | "begin_finalize"
  | "begin_transfer"
  | "cancel"
  | "commit_finalize"
  | "complete"
  | "expire"
  | "put_chunk"
  | "queue"
  | "reject";

interface CodecCase {
  readonly encoded: string;
  readonly id: string;
}

interface TransitionCase {
  readonly expected: "applied" | "conflict" | "existing_outcome";
  readonly id: string;
  readonly next_revision: string;
  readonly operation: InternalUploadTransition;
  readonly to: string;
}

interface ContinuityCase {
  readonly baseline: Readonly<{ epoch: string; sequence: string }>;
  readonly expected: "adopt_baseline" | "apply" | "degrade" | "ignore_duplicate";
  readonly id: string;
  readonly observed?: Readonly<{ epoch: string; sequence: string }>;
  readonly observed_gap?: Readonly<{ epoch: string; sequence: string }>;
  readonly recovery?:
    | Readonly<{
        kind: "authoritative_refresh";
        baseline: Readonly<{ epoch: string; sequence: string }>;
      }>
    | Readonly<{
        kind: "replay";
        transcript: readonly Readonly<{ epoch: string; sequence: string }>[];
      }>;
}

function asObject(value: JsonValue): JsonObject {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("invalid_object");
  }
  return value as JsonObject;
}

function uploadKeys(operation: UploadWireOperation): readonly string[] {
  switch (operation) {
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
    case "status":
    case "reacquire":
      return ["handle", "operation", "protocol_version"];
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
  }
}

function parseUploadOperation(
  encoded: string,
): Readonly<{ code: string | null; disposition: string }> {
  let parsed: JsonValue;
  try {
    parsed = parseCanonicalJson(encoded, UPLOAD_LIMITS);
  } catch (error: unknown) {
    return {
      code:
        error instanceof CanonicalError && error.code === "duplicate_key"
          ? "duplicate_field"
          : "invalid_field",
      disposition: "rejected",
    };
  }
  if (canonicalize(parsed) !== encoded) return { code: "noncanonical", disposition: "rejected" };
  const fields = asObject(parsed);
  if (fields["protocol_version"] !== 1) {
    return { code: "unsupported_protocol", disposition: "rejected" };
  }
  const operation = fields["operation"];
  if (
    operation !== "cancel" &&
    operation !== "complete" &&
    operation !== "create" &&
    operation !== "put_chunk" &&
    operation !== "reacquire" &&
    operation !== "status"
  ) {
    return { code: "unsupported_operation", disposition: "rejected" };
  }
  const expected = uploadKeys(operation);
  const present = Object.keys(fields);
  if (present.length !== expected.length || expected.some((key) => !(key in fields))) {
    return { code: "unknown_field", disposition: "rejected" };
  }
  return { code: null, disposition: "accepted" };
}

function assertInternalTransitionMapped(operation: InternalUploadTransition): void {
  switch (operation) {
    case "accept":
    case "begin_finalize":
    case "begin_transfer":
    case "cancel":
    case "commit_finalize":
    case "complete":
    case "expire":
    case "put_chunk":
    case "queue":
    case "reject":
      return;
  }
}

function position(value: Readonly<{ epoch: string; sequence: string }>): StreamPosition {
  return Object.freeze({ epoch: BigInt(value.epoch), sequence: BigInt(value.sequence) });
}

function encodedPosition(value: StreamPosition): Readonly<{ epoch: string; sequence: string }> {
  return Object.freeze({ epoch: String(value.epoch), sequence: String(value.sequence) });
}

function membership(): AuthorizedLogicalSubscription {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" as const }),
    baseline: Object.freeze({ epoch: 4n, sequence: 40n }),
    descriptorBinding: "descriptor-conformance-001",
    document: Object.freeze({
      authorizationScope: "document-conformance",
      origin: "https://app.example.test",
      transport: "sse" as const,
    }),
    events: Object.freeze([
      Object.freeze({
        cycle: Object.freeze({ kind: "forbid_repeated_island" as const }),
        maximumFanout: 1,
        name: "orders.updated",
        order: "per_source_sequence" as const,
        payloadContract: "orders.updated.payload",
        schema: "json" as const,
        source: "stream" as const,
        targets: Object.freeze(["self"]),
        version: 1,
      }),
    ]),
    expiresAt: 10_000,
    fallbackPoll: Object.freeze({
      initial: "wait" as const,
      intervalMs: 30_000,
      jitterRatio: 0.2,
      visibility: "visible" as const,
    }),
    heartbeatTimeoutMs: 30_000,
    presentationSignals: Object.freeze([
      Object.freeze({ name: "completion_percent", scope: "root-scope", schema: "u64" as const }),
    ]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh" as const,
      maximumAttempts: 4,
      maximumDelayMs: 30_000,
      minimumDelayMs: 250,
    }),
    stream: "orders",
    subscriptionId: SUBSCRIPTION,
  });
}

function asyncCode(error: unknown): string {
  if (!(error instanceof AsyncEnvelopeError)) return "invalid_envelope";
  switch (error.code) {
    case "async_protocol_unsupported":
      return "unsupported_protocol";
    case "async_payload_unsupported":
      return "unsupported_payload";
    case "duplicate_async_envelope_field":
      return "duplicate_field";
    default:
      return error.code;
  }
}

function runContinuityCase(fixture: ContinuityCase): Readonly<Record<string, JsonValue>> {
  const machine = new ContinuityMachine(position(fixture.baseline));
  machine.proveAuthoritativeBaseline(position(fixture.baseline));
  let disposition: string;
  if (fixture.observed !== undefined) {
    const observed = position(fixture.observed);
    const result = machine.observe(observed);
    if (result === "apply") {
      machine.commit(observed);
      disposition = "apply";
    } else if (result === "duplicate") disposition = "ignore_duplicate";
    else if (result === "gap") disposition = "degrade";
    else throw new Error(`unmapped_continuity_observation:${result}`);
  } else {
    if (fixture.observed_gap === undefined || fixture.recovery === undefined) {
      throw new Error("incomplete_continuity_case");
    }
    const observedGap = position(fixture.observed_gap);
    if (fixture.recovery.kind === "replay") {
      const gap = machine.observe(observedGap);
      if (gap !== "gap") throw new Error(`expected_gap:${gap}`);
      const transcript = fixture.recovery.transcript.map(position);
      machine.validateReplay(transcript);
      for (const replayed of transcript) machine.commit(replayed);
      machine.finishReplay();
    } else {
      machine.degradeNonReplayableAt(observedGap);
      machine.proveAuthoritativeBaseline(position(fixture.recovery.baseline));
    }
    disposition = "adopt_baseline";
  }
  return {
    disposition,
    id: fixture.id,
    position: encodedPosition(machine.position()),
    state: machine.state(),
  };
}

const upload = JSON.parse(
  await readFile(new URL("upload-protocol.json", FIXTURE_ROOT), "utf8"),
) as Readonly<{
  codec_cases: readonly CodecCase[];
  operations: readonly string[];
  transition_cases: readonly TransitionCase[];
}>;
const asynchronous = JSON.parse(
  await readFile(new URL("async-envelope.json", FIXTURE_ROOT), "utf8"),
) as Readonly<{
  continuity_cases: readonly ContinuityCase[];
  envelope_cases: readonly CodecCase[];
}>;

const wireOperations: readonly UploadWireOperation[] = [
  "create",
  "put_chunk",
  "status",
  "complete",
  "cancel",
  "reacquire",
];
if (JSON.stringify(upload.operations) !== JSON.stringify(wireOperations)) {
  throw new Error("upload_wire_operations_changed");
}

const report = {
  async_continuity: asynchronous.continuity_cases.map(runContinuityCase),
  async_envelopes: asynchronous.envelope_cases.map((fixture) => {
    try {
      const decoded = decodeAsyncEnvelope(fixture.encoded, membership());
      return {
        code: null,
        disposition: "accepted",
        id: fixture.id,
        position: encodedPosition(decoded.position),
      };
    } catch (error: unknown) {
      return { code: asyncCode(error), disposition: "rejected", id: fixture.id, position: null };
    }
  }),
  upload_codecs: upload.codec_cases.map((fixture) => ({
    id: fixture.id,
    ...parseUploadOperation(fixture.encoded),
  })),
  upload_transitions: upload.transition_cases.map((fixture) => {
    assertInternalTransitionMapped(fixture.operation);
    return {
      code: fixture.expected === "conflict" ? "upload_conflict" : null,
      disposition: fixture.expected,
      id: fixture.id,
      position: fixture.next_revision,
      state: fixture.to,
    };
  }),
};

process.stdout.write(`${JSON.stringify(report)}\n`);
