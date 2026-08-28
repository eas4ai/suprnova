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
import { FIXTURE_FILES_V4, loadFixtureSet } from "../../src/conformance.js";
import { BoundedOwner } from "../../src/features/bounded.js";
import {
  createOptionalFeatureDriver,
  defineAsyncFeature,
  defineUploadsFeature,
  RUNTIME_FEATURE_CORE_RANGE,
  type RuntimeFeature,
} from "../../src/features/contract.js";
import { RuntimeDiagnostics } from "../../src/runtime/diagnostics.js";

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
  | "fail"
  | "put_chunk"
  | "queue"
  | "reject";

interface CodecCase {
  readonly encoded: string;
  readonly id: string;
}

interface TransitionCase {
  readonly currentRevision: bigint | null;
  readonly expectedRevision: bigint;
  readonly id: string;
  readonly idempotencyKey: string;
  readonly operation: InternalUploadTransition;
  readonly retry: boolean;
  readonly state: UploadState;
}

type UploadState =
  | "canceled"
  | "created"
  | "expired"
  | "failed"
  | "finalized"
  | "finalizing"
  | "queued"
  | "ready"
  | "rejected"
  | "transferring"
  | "verifying";

interface SignalNameCase {
  readonly expected: "accepted" | "rejected";
  readonly value: string;
}

interface CompatibilityCase {
  readonly capability_version: number | null;
  readonly core_version: string;
  readonly feature: string;
  readonly id: string;
  readonly present: boolean;
}

interface RuntimeFeatureContract {
  readonly capability_version: number;
  readonly compatible_core: Readonly<{ maximum_exclusive: string; minimum: string }>;
  readonly name: string;
}

interface RedactionCase {
  readonly class: string;
  readonly id: string;
  readonly sample: JsonValue;
}

interface ResourceBounds {
  readonly max_active: number;
  readonly max_bytes: number;
  readonly max_items: number;
}

interface ResourceCase {
  readonly id: string;
  readonly operations: readonly unknown[];
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

function assertNever(value: never): never {
  throw new Error(`unmapped_closed_value:${String(value)}`);
}

function internalUploadTransition(value: unknown): InternalUploadTransition {
  switch (value) {
    case "accept":
    case "begin_finalize":
    case "begin_transfer":
    case "cancel":
    case "commit_finalize":
    case "complete":
    case "expire":
    case "fail":
    case "put_chunk":
    case "queue":
    case "reject":
      return value;
    default:
      throw new Error("unknown_internal_upload_transition");
  }
}

function uploadState(value: unknown): UploadState {
  switch (value) {
    case "canceled":
    case "created":
    case "expired":
    case "failed":
    case "finalized":
    case "finalizing":
    case "queued":
    case "ready":
    case "rejected":
    case "transferring":
    case "verifying":
      return value;
    default:
      throw new Error("unknown_upload_state");
  }
}

function transitionCase(value: unknown): TransitionCase {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("invalid_transition_case");
  }
  const fields = value as Record<string, unknown>;
  if (
    typeof fields["id"] !== "string" ||
    typeof fields["expected_revision"] !== "string" ||
    (fields["current_revision"] !== undefined && typeof fields["current_revision"] !== "string") ||
    (fields["idempotency_key"] !== undefined && typeof fields["idempotency_key"] !== "string")
  ) {
    throw new Error("invalid_transition_case");
  }
  return Object.freeze({
    currentRevision:
      typeof fields["current_revision"] === "string" ? BigInt(fields["current_revision"]) : null,
    expectedRevision: BigInt(fields["expected_revision"]),
    id: fields["id"],
    idempotencyKey:
      typeof fields["idempotency_key"] === "string" ? fields["idempotency_key"] : fields["id"],
    operation: internalUploadTransition(fields["operation"]),
    retry: fields["retry"] !== undefined,
    state: uploadState(fields["from"]),
  });
}

function terminalUploadState(state: UploadState): boolean {
  return (
    state === "canceled" ||
    state === "expired" ||
    state === "failed" ||
    state === "finalized" ||
    state === "rejected"
  );
}

function nextUploadState(state: UploadState, operation: InternalUploadTransition): UploadState {
  if (terminalUploadState(state)) throw new Error("invalid_upload_transition");
  switch (operation) {
    case "queue":
      if (state === "created") return "queued";
      break;
    case "begin_transfer":
      if (state === "queued") return "transferring";
      break;
    case "put_chunk":
      if (state === "transferring") return "transferring";
      break;
    case "complete":
      if (state === "transferring") return "verifying";
      break;
    case "accept":
      if (state === "verifying") return "ready";
      break;
    case "begin_finalize":
      if (state === "ready") return "finalizing";
      break;
    case "commit_finalize":
      if (state === "finalizing") return "finalized";
      break;
    case "cancel":
      if (
        state === "created" ||
        state === "queued" ||
        state === "ready" ||
        state === "transferring" ||
        state === "verifying"
      ) {
        return "canceled";
      }
      break;
    case "reject":
      if (state === "verifying") return "rejected";
      break;
    case "expire":
      if (
        state === "created" ||
        state === "queued" ||
        state === "ready" ||
        state === "transferring" ||
        state === "verifying"
      ) {
        return "expired";
      }
      break;
    case "fail":
      return "failed";
    default:
      return assertNever(operation);
  }
  throw new Error("invalid_upload_transition");
}

function runUploadTransition(fixture: TransitionCase): Readonly<Record<string, JsonValue>> {
  let state = fixture.state;
  let revision = fixture.currentRevision ?? fixture.expectedRevision;
  const outcomes = new Map<
    string,
    Readonly<{ operation: InternalUploadTransition; revision: bigint; state: UploadState }>
  >();

  const apply = (): Readonly<{
    code: string | null;
    disposition: "applied" | "conflict" | "existing_outcome" | "rejected";
  }> => {
    const existing = outcomes.get(fixture.idempotencyKey);
    if (existing !== undefined) {
      if (existing.operation !== fixture.operation) {
        return { code: "idempotency_conflict", disposition: "rejected" };
      }
      state = existing.state;
      revision = existing.revision;
      return { code: null, disposition: "existing_outcome" };
    }
    if (fixture.expectedRevision !== revision) {
      return { code: "upload_conflict", disposition: "conflict" };
    }
    try {
      state = nextUploadState(state, fixture.operation);
    } catch {
      return { code: "invalid_upload_transition", disposition: "rejected" };
    }
    revision += 1n;
    outcomes.set(
      fixture.idempotencyKey,
      Object.freeze({ operation: fixture.operation, revision, state }),
    );
    return { code: null, disposition: "applied" };
  };

  const first = apply();
  const outcome = fixture.retry && first.disposition === "applied" ? apply() : first;
  return {
    code: outcome.code,
    disposition: outcome.disposition,
    id: fixture.id,
    position: String(revision),
    state,
  };
}

function position(value: Readonly<{ epoch: string; sequence: string }>): StreamPosition {
  return Object.freeze({ epoch: BigInt(value.epoch), sequence: BigInt(value.sequence) });
}

function encodedPosition(value: StreamPosition): Readonly<{ epoch: string; sequence: string }> {
  return Object.freeze({ epoch: String(value.epoch), sequence: String(value.sequence) });
}

function membership(signalName = "completion_percent"): AuthorizedLogicalSubscription {
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
      Object.freeze({ name: signalName, scope: "root-scope", schema: "u64" as const }),
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

const EXPECTED_V4_INVENTORY = Object.freeze({
  "async-envelope.json": [
    "codec_limits",
    "continuity_cases",
    "envelope_cases",
    "live_protocol_versions",
    "payload_kinds",
    "protocol_versions",
    "schema_version",
    "signal_name_cases",
    "subscription_states",
  ],
  "compatibility.json": [
    "cases",
    "compatible_core",
    "live_protocol_versions",
    "schema_version",
    "snapshot_versions",
  ],
  "diagnostics.json": [
    "allowed_dimensions",
    "codes",
    "phases",
    "redacted_classes",
    "redaction_cases",
    "retention",
    "schema_version",
    "severities",
  ],
  "directive-grammar.json": [
    "contract_version",
    "directives",
    "event_modifiers",
    "feedback_modifiers",
    "freshness_combinations",
    "model_modifiers",
    "morph_modifiers",
    "navigation_modifiers",
    "reserved",
    "schema_version",
    "syntax",
    "transition_modifiers",
  ],
  "resource-lifecycle.json": ["bounds", "cases", "resource_kinds", "schema_version", "states"],
  "runtime-features.json": [
    "allowed_island_operations",
    "features",
    "forbidden_island_operations",
    "registration_outcomes",
    "registry",
    "retirement",
    "schema_version",
  ],
  "upload-protocol.json": [
    "codec_cases",
    "codec_limits",
    "live_protocol_versions",
    "operations",
    "presentation_states",
    "protocol_versions",
    "schema_version",
    "states",
    "terminal_states",
    "transition_cases",
  ],
} as const);

function fixtureRecord(
  fixtures: ReadonlyMap<string, unknown>,
  name: string,
): Record<string, unknown> {
  const value = fixtures.get(name);
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid_v4_fixture:${name}`);
  }
  return value as Record<string, unknown>;
}

function fixtureArray(record: Record<string, unknown>, name: string): readonly unknown[] {
  const value = record[name];
  if (!Array.isArray(value)) throw new Error(`invalid_v4_collection:${name}`);
  return value;
}

function v4Inventory(fixtures: ReadonlyMap<string, unknown>): Record<string, readonly string[]> {
  const names = [...fixtures.keys()].sort();
  if (JSON.stringify(names) !== JSON.stringify([...FIXTURE_FILES_V4].sort())) {
    throw new Error("v4_fixture_file_inventory_changed");
  }
  const inventory = Object.fromEntries(
    names.map((name) => [name, Object.keys(fixtureRecord(fixtures, name)).sort()]),
  );
  if (JSON.stringify(inventory) !== JSON.stringify(EXPECTED_V4_INVENTORY)) {
    throw new Error("v4_fixture_collection_inventory_changed");
  }
  return inventory;
}

function runSignalNameCase(
  fixture: SignalNameCase,
  presentationTemplate: CodecCase,
): Readonly<Record<string, JsonValue>> {
  const template = asObject(parseCanonicalJson(presentationTemplate.encoded, UPLOAD_LIMITS));
  const payload = asObject(template["payload"] as JsonValue);
  const candidate = Object.freeze({
    ...template,
    payload: Object.freeze({ ...payload, name: fixture.value }),
  }) as JsonObject;
  try {
    decodeAsyncEnvelope(canonicalize(candidate), membership(fixture.value));
    return { code: null, disposition: "accepted", id: fixture.value };
  } catch {
    return { code: "invalid_signal_name", disposition: "rejected", id: fixture.value };
  }
}

function version(value: string): readonly [number, number, number] {
  const match = /^(\d+)\.(\d+)\.(\d+)$/u.exec(value);
  if (match === null) throw new Error("invalid_fixture_semver");
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compareVersion(left: string, right: string): number {
  const leftParts = version(left);
  const rightParts = version(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    const delta = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (delta !== 0) return delta;
  }
  return 0;
}

function actualFeature(name: string): RuntimeFeature | null {
  const definition = {
    connectDocument() {
      return { connectIsland: () => undefined, dispose: () => undefined };
    },
  };
  if (name === "uploads") return defineUploadsFeature(definition);
  if (name === "async") return defineAsyncFeature(definition);
  return null;
}

function runCompatibilityCase(
  fixture: CompatibilityCase,
  contracts: readonly RuntimeFeatureContract[],
): Readonly<Record<string, JsonValue>> {
  if (!fixture.present) {
    return { code: null, disposition: "ordinary_live_available", id: fixture.id };
  }
  const contract = contracts.find((candidate) => candidate.name === fixture.feature);
  const feature = actualFeature(fixture.feature);
  if (contract === undefined || feature === null) {
    return { code: "feature_unavailable", disposition: "feature_unavailable", id: fixture.id };
  }
  const coreCompatible =
    compareVersion(fixture.core_version, contract.compatible_core.minimum) >= 0 &&
    compareVersion(fixture.core_version, contract.compatible_core.maximum_exclusive) < 0;
  const candidate = [...feature] as unknown[];
  candidate[2] = fixture.capability_version;
  candidate[3] = coreCompatible ? RUNTIME_FEATURE_CORE_RANGE : 0;
  const registration = createOptionalFeatureDriver().register(
    Object.freeze(candidate) as unknown as RuntimeFeature,
  );
  const compatible =
    fixture.capability_version === contract.capability_version && registration === "registered";
  return {
    code: compatible ? null : "feature_unavailable",
    disposition: compatible ? "compatible" : "feature_unavailable",
    id: fixture.id,
  };
}

function runRedactionCase(fixture: RedactionCase): Readonly<Record<string, JsonValue>> {
  const diagnostics = new RuntimeDiagnostics({ maxEntries: 1, mode: "verbose" });
  diagnostics.record(
    {
      code: "configuration_invalid",
      detailCode: "invalid_shape",
      phase: "configuration",
      severity: "error",
    },
    fixture.sample,
  );
  const serialized = JSON.stringify(diagnostics.entries());
  const unsafe =
    typeof fixture.sample === "string" ? fixture.sample : JSON.stringify(fixture.sample);
  const redacted = !serialized.includes(unsafe);
  return {
    code: redacted ? null : "diagnostic_value_leaked",
    disposition: redacted ? "redacted" : "rejected",
    id: fixture.id,
    state: redacted ? "[redacted]" : null,
  };
}

function runResourceCase(
  fixture: ResourceCase,
  bounds: ResourceBounds,
): Readonly<Record<string, JsonValue>>[] {
  const owner = new BoundedOwner<string>({
    maxActive: bounds.max_active,
    maxBytes: bounds.max_bytes,
    maxItems: bounds.max_items,
  });
  let lease: Readonly<{ dispose(): void }> | null = null;
  return fixture.operations.map((value, index) => {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("invalid_resource_operation");
    }
    const operation = value as Record<string, unknown>;
    let outcome: JsonValue;
    switch (operation["operation"]) {
      case "enqueue": {
        if (typeof operation["bytes"] !== "number") throw new Error("invalid_resource_bytes");
        outcome = owner.enqueue(`item-${String(index)}`, operation["bytes"]);
        break;
      }
      case "acquire":
        lease = owner.acquire();
        outcome = lease === null ? owner.snapshot().state : "acquired";
        break;
      case "release":
        if (lease === null) throw new Error("missing_resource_lease");
        lease.dispose();
        lease = null;
        outcome = "released";
        break;
      case "suspend":
        outcome = owner.suspend();
        break;
      case "resume":
        outcome = owner.resume();
        break;
      case "retire": {
        const canceled = !owner.snapshot().canceled;
        const retirement = owner.retire();
        lease = null;
        outcome = {
          canceled,
          drained_bytes: retirement.drainedBytes,
          drained_items: retirement.drainedItems,
          released_permits: retirement.releasedPermits,
        };
        break;
      }
      default:
        throw new Error("unknown_resource_operation");
    }
    return {
      code: null,
      disposition: typeof outcome === "string" ? outcome : "retired",
      id: `${fixture.id}:${String(index)}`,
      outcome,
      position: String(index),
      state: owner.snapshot().state,
    };
  });
}

const fixtures = await loadFixtureSet(4);
const inventory = v4Inventory(fixtures);
const upload = fixtureRecord(fixtures, "upload-protocol.json");
const asynchronous = fixtureRecord(fixtures, "async-envelope.json");
const compatibility = fixtureRecord(fixtures, "compatibility.json");
const diagnostics = fixtureRecord(fixtures, "diagnostics.json");
const resources = fixtureRecord(fixtures, "resource-lifecycle.json");
const runtimeFeatures = fixtureRecord(fixtures, "runtime-features.json");
// Loading this reviewed collection is intentional even though checker suites execute its grammar.
fixtureRecord(fixtures, "directive-grammar.json");

const codecCases = fixtureArray(upload, "codec_cases") as readonly CodecCase[];
const transitionCases = fixtureArray(upload, "transition_cases").map(transitionCase);
const envelopeCases = fixtureArray(asynchronous, "envelope_cases") as readonly CodecCase[];
const continuityCases = fixtureArray(asynchronous, "continuity_cases") as readonly ContinuityCase[];
const signalCases = fixtureArray(asynchronous, "signal_name_cases") as readonly SignalNameCase[];
const presentationTemplate = envelopeCases.find((fixture) => fixture.id === "presentation-signal");
if (presentationTemplate === undefined) throw new Error("missing_presentation_signal_template");

let rejectedUnknownTransition = false;
try {
  internalUploadTransition("future_internal_transition");
} catch {
  rejectedUnknownTransition = true;
}
if (!rejectedUnknownTransition) throw new Error("unknown_internal_transition_was_accepted");

const wireOperations: readonly UploadWireOperation[] = [
  "create",
  "put_chunk",
  "status",
  "complete",
  "cancel",
  "reacquire",
];
if (JSON.stringify(upload["operations"]) !== JSON.stringify(wireOperations)) {
  throw new Error("upload_wire_operations_changed");
}

const report = {
  async_continuity: continuityCases.map(runContinuityCase),
  async_envelopes: envelopeCases.map((fixture) => {
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
  async_signals: signalCases.map((fixture) => runSignalNameCase(fixture, presentationTemplate)),
  compatibility: (fixtureArray(compatibility, "cases") as readonly CompatibilityCase[]).map(
    (fixture) =>
      runCompatibilityCase(
        fixture,
        fixtureArray(runtimeFeatures, "features") as readonly RuntimeFeatureContract[],
      ),
  ),
  diagnostics: (fixtureArray(diagnostics, "redaction_cases") as readonly RedactionCase[]).map(
    runRedactionCase,
  ),
  inventory,
  resource_lifecycle: (fixtureArray(resources, "cases") as readonly ResourceCase[]).flatMap(
    (fixture) => runResourceCase(fixture, resources["bounds"] as ResourceBounds),
  ),
  upload_codecs: codecCases.map((fixture) => ({
    id: fixture.id,
    ...parseUploadOperation(fixture.encoded),
  })),
  upload_transitions: transitionCases.map(runUploadTransition),
};

process.stdout.write(`${JSON.stringify(report)}\n`);
