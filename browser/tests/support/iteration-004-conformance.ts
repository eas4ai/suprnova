import { AsyncEnvelopeError, decodeAsyncEnvelope } from "../../src/async-updates/envelope.js";
import { ContinuityMachine } from "../../src/async-updates/continuity.js";
import type {
  AuthorizedLogicalSubscription,
  StreamPosition,
} from "../../src/async-updates/types.js";
import {
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
import {
  decodeUploadProtocolOperation,
  UploadProtocolError,
  type UploadWireOperation,
} from "../../src/uploads/protocol.js";
import {
  parseUploadProtocolState,
  parseUploadProtocolTransition,
  UploadProtocolStateError,
  UploadProtocolStateMachine,
  type UploadProtocolState,
  type UploadProtocolTransition,
} from "../../src/uploads/state.js";

const SUBSCRIPTION = "c3Vic2NyaXB0aW9uLTAwMQ";
const UPLOAD_LIMITS = Object.freeze({
  maxBytes: 16_384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 4_096,
});

interface CodecCase {
  readonly encoded: string;
  readonly expected: string;
  readonly id: string;
}

interface TransitionCase {
  readonly currentRevision: bigint | null;
  readonly expectedRevision: bigint;
  readonly expectedDisposition: "applied" | "conflict" | "existing_outcome";
  readonly id: string;
  readonly idempotencyKey: string;
  readonly nextRevision: bigint;
  readonly operation: UploadProtocolTransition;
  readonly retry: boolean;
  readonly state: UploadProtocolState;
  readonly to: UploadProtocolState;
}

interface SignalNameCase {
  readonly expected: "accepted" | "rejected";
  readonly value: string;
}

interface CompatibilityCase {
  readonly capability_version: number | null;
  readonly core_version: string;
  readonly expected: "compatible" | "feature_unavailable" | "ordinary_live_available";
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
  readonly expected: "[redacted]";
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
  readonly state: "current" | "degraded";
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

function assertFixtureOracle(actual: unknown, expected: unknown, path: string): void {
  if (expected !== null && typeof expected === "object") {
    if (Array.isArray(expected)) {
      if (!Array.isArray(actual) || actual.length !== expected.length) {
        throw new Error(`fixture_oracle_array_mismatch:${path}`);
      }
      for (let index = 0; index < expected.length; index += 1) {
        assertFixtureOracle(actual[index], expected[index], `${path}[${String(index)}]`);
      }
      return;
    }
    if (actual === null || typeof actual !== "object" || Array.isArray(actual)) {
      throw new Error(`fixture_oracle_object_mismatch:${path}`);
    }
    for (const [field, value] of Object.entries(expected)) {
      if (!Object.prototype.hasOwnProperty.call(actual, field)) {
        throw new Error(`fixture_oracle_field_missing:${path}.${field}`);
      }
      assertFixtureOracle((actual as Record<string, unknown>)[field], value, `${path}.${field}`);
    }
    return;
  }
  if (!Object.is(actual, expected)) {
    throw new Error(`fixture_oracle_value_mismatch:${path}`);
  }
}

function parseUploadOperation(
  encoded: string,
): Readonly<{ code: string | null; disposition: string }> {
  try {
    decodeUploadProtocolOperation(encoded);
    return { code: null, disposition: "accepted" };
  } catch (error: unknown) {
    return {
      code: error instanceof UploadProtocolError ? error.code : "invalid_field",
      disposition: "rejected",
    };
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
    typeof fields["next_revision"] !== "string" ||
    (fields["expected"] !== "applied" &&
      fields["expected"] !== "conflict" &&
      fields["expected"] !== "existing_outcome") ||
    (fields["current_revision"] !== undefined && typeof fields["current_revision"] !== "string") ||
    (fields["idempotency_key"] !== undefined && typeof fields["idempotency_key"] !== "string")
  ) {
    throw new Error("invalid_transition_case");
  }
  return Object.freeze({
    currentRevision:
      typeof fields["current_revision"] === "string" ? BigInt(fields["current_revision"]) : null,
    expectedDisposition: fields["expected"],
    expectedRevision: BigInt(fields["expected_revision"]),
    id: fields["id"],
    idempotencyKey:
      typeof fields["idempotency_key"] === "string" ? fields["idempotency_key"] : fields["id"],
    nextRevision: BigInt(fields["next_revision"]),
    operation: parseUploadProtocolTransition(fields["operation"]),
    retry: fields["retry"] !== undefined,
    state: parseUploadProtocolState(fields["from"]),
    to: parseUploadProtocolState(fields["to"]),
  });
}

function runUploadTransition(fixture: TransitionCase): Readonly<Record<string, JsonValue>> {
  const request = Object.freeze({
    expectedRevision: fixture.expectedRevision,
    idempotencyKey: fixture.idempotencyKey,
    transition: fixture.operation,
  });
  let machine = new UploadProtocolStateMachine(
    fixture.state,
    fixture.currentRevision ?? fixture.expectedRevision,
  );
  let outcome:
    | Readonly<{ code: string | null; disposition: string }>
    | ReturnType<UploadProtocolStateMachine["apply"]>;
  try {
    if (fixture.retry) {
      machine = new UploadProtocolStateMachine(fixture.state, fixture.expectedRevision);
      machine.apply(request);
    }
    outcome = machine.apply(request);
  } catch (error: unknown) {
    const code =
      error instanceof UploadProtocolStateError ? error.code : "invalid_upload_transition";
    outcome = {
      code,
      disposition: code === "upload_conflict" ? "conflict" : "rejected",
    };
  }
  return {
    code: "code" in outcome ? outcome.code : null,
    disposition: outcome.disposition,
    id: fixture.id,
    position: String(machine.revision),
    state: machine.state,
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
const compatibilityCases = fixtureArray(compatibility, "cases") as readonly CompatibilityCase[];
const redactionCases = fixtureArray(diagnostics, "redaction_cases") as readonly RedactionCase[];
const resourceCases = fixtureArray(resources, "cases") as readonly ResourceCase[];
const presentationTemplate = envelopeCases.find((fixture) => fixture.id === "presentation-signal");
if (presentationTemplate === undefined) throw new Error("missing_presentation_signal_template");

let rejectedUnknownTransition = false;
try {
  parseUploadProtocolTransition("future_internal_transition");
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
  compatibility: compatibilityCases.map((fixture) =>
    runCompatibilityCase(
      fixture,
      fixtureArray(runtimeFeatures, "features") as readonly RuntimeFeatureContract[],
    ),
  ),
  diagnostics: redactionCases.map(runRedactionCase),
  inventory,
  resource_lifecycle: resourceCases.flatMap((fixture) =>
    runResourceCase(fixture, resources["bounds"] as ResourceBounds),
  ),
  upload_codecs: codecCases.map((fixture) => ({
    id: fixture.id,
    ...parseUploadOperation(fixture.encoded),
  })),
  upload_transitions: transitionCases.map(runUploadTransition),
};

for (const [index, fixture] of codecCases.entries()) {
  assertFixtureOracle(
    report.upload_codecs[index],
    {
      code: fixture.expected === "accepted" ? null : fixture.expected,
      disposition: fixture.expected === "accepted" ? "accepted" : "rejected",
      id: fixture.id,
    },
    `upload_codecs.${fixture.id}`,
  );
}
for (const [index, fixture] of transitionCases.entries()) {
  assertFixtureOracle(
    report.upload_transitions[index],
    {
      code: fixture.expectedDisposition === "conflict" ? "upload_conflict" : null,
      disposition: fixture.expectedDisposition,
      id: fixture.id,
      position: String(fixture.nextRevision),
      state: fixture.to,
    },
    `upload_transitions.${fixture.id}`,
  );
}
for (const [index, fixture] of envelopeCases.entries()) {
  assertFixtureOracle(
    report.async_envelopes[index],
    {
      code: fixture.expected === "accepted" ? null : fixture.expected,
      disposition: fixture.expected === "accepted" ? "accepted" : "rejected",
      id: fixture.id,
    },
    `async_envelopes.${fixture.id}`,
  );
}
for (const [index, fixture] of signalCases.entries()) {
  assertFixtureOracle(
    report.async_signals[index],
    {
      code: fixture.expected === "accepted" ? null : "invalid_signal_name",
      disposition: fixture.expected,
      id: fixture.value,
    },
    `async_signals.${fixture.value}`,
  );
}
for (const [index, fixture] of continuityCases.entries()) {
  assertFixtureOracle(
    report.async_continuity[index],
    { disposition: fixture.expected, id: fixture.id, state: fixture.state },
    `async_continuity.${fixture.id}`,
  );
}
for (const [index, fixture] of compatibilityCases.entries()) {
  assertFixtureOracle(
    report.compatibility[index],
    {
      code: fixture.expected === "feature_unavailable" ? "feature_unavailable" : null,
      disposition: fixture.expected,
      id: fixture.id,
    },
    `compatibility.${fixture.id}`,
  );
}
for (const [index, fixture] of redactionCases.entries()) {
  assertFixtureOracle(
    report.diagnostics[index],
    { code: null, disposition: "redacted", id: fixture.id, state: fixture.expected },
    `diagnostics.${fixture.id}`,
  );
}
let resourceIndex = 0;
for (const fixture of resourceCases) {
  for (const [operationIndex, value] of fixture.operations.entries()) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("invalid_resource_operation_oracle");
    }
    const expected = (value as Record<string, unknown>)["expected"];
    assertFixtureOracle(
      report.resource_lifecycle[resourceIndex],
      { id: `${fixture.id}:${String(operationIndex)}`, outcome: expected },
      `resource_lifecycle.${fixture.id}.${String(operationIndex)}`,
    );
    resourceIndex += 1;
  }
}

process.stdout.write(`${JSON.stringify(report)}\n`);
