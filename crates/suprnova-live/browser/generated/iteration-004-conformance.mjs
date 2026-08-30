// @generated from iteration-004-conformance.ts; do not edit.

// src/canonical.ts
var DEFAULT_CANONICAL_LIMITS = {
  maxBytes: 64 * 1024,
  maxDepth: 32,
  maxEntries: 2048,
  maxStringBytes: 16 * 1024
};
var CanonicalError = class extends Error {
  constructor(code) {
    super(code);
    this.code = code;
    this.name = "CanonicalError";
  }
};
var Parser = class {
  constructor(text, limits) {
    this.text = text;
    this.limits = limits;
    this.index = 0;
    this.entries = 0;
    this.bytes = new TextEncoder().encode(text).byteLength;
    if (this.bytes > limits.maxBytes) throw new CanonicalError("input_too_large");
  }
  parse() {
    this.space();
    const value = this.value(0);
    this.space();
    if (this.index !== this.text.length) throw new CanonicalError("invalid_json");
    return value;
  }
  value(depth) {
    const current = this.text[this.index];
    if ((current === "{" || current === "[") && depth >= this.limits.maxDepth) {
      throw new CanonicalError("input_too_deep");
    }
    if (current === '"') return this.string();
    if (current === "{") return this.object(depth + 1);
    if (current === "[") return this.array(depth + 1);
    if (this.text.startsWith("true", this.index)) return this.literal("true", true);
    if (this.text.startsWith("false", this.index)) return this.literal("false", false);
    if (this.text.startsWith("null", this.index)) return this.literal("null", null);
    return this.number();
  }
  literal(token, value) {
    this.index += token.length;
    return value;
  }
  string() {
    const start = this.index;
    this.index += 1;
    let escaped = false;
    while (this.index < this.text.length) {
      const character = this.text[this.index];
      if (character === void 0) break;
      if (!escaped && character === '"') {
        this.index += 1;
        const raw = this.text.slice(start, this.index);
        let decoded;
        try {
          decoded = JSON.parse(raw);
        } catch {
          throw new CanonicalError("invalid_json");
        }
        if (typeof decoded !== "string") throw new CanonicalError("invalid_json");
        if (hasLoneSurrogate(decoded)) throw new CanonicalError("invalid_json");
        if (new TextEncoder().encode(decoded).byteLength > this.limits.maxStringBytes) {
          throw new CanonicalError("string_too_long");
        }
        return decoded;
      }
      if (!escaped && character.charCodeAt(0) < 32) {
        throw new CanonicalError("invalid_json");
      }
      if (escaped) escaped = false;
      else if (character === "\\") escaped = true;
      this.index += 1;
    }
    throw new CanonicalError("invalid_json");
  }
  number() {
    const match = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/u.exec(this.text.slice(this.index));
    const token = match?.[0];
    if (token === void 0) throw new CanonicalError("invalid_json");
    this.index += token.length;
    const value = Number(token);
    if (!Number.isFinite(value)) throw new CanonicalError("invalid_number");
    if (!token.includes(".") && !/[eE]/u.test(token) && !Number.isSafeInteger(value)) {
      throw new CanonicalError("invalid_number");
    }
    return Object.is(value, -0) ? 0 : value;
  }
  array(depth) {
    this.index += 1;
    this.space();
    const values = [];
    if (this.text[this.index] === "]") {
      this.index += 1;
      return values;
    }
    for (; ; ) {
      const value = this.value(depth);
      this.bumpEntry();
      values.push(value);
      this.space();
      const separator = this.text[this.index];
      this.index += 1;
      if (separator === "]") return values;
      if (separator !== ",") throw new CanonicalError("invalid_json");
      this.space();
    }
  }
  object(depth) {
    this.index += 1;
    this.space();
    const values = /* @__PURE__ */ Object.create(null);
    const keys = /* @__PURE__ */ new Set();
    if (this.text[this.index] === "}") {
      this.index += 1;
      return values;
    }
    for (; ; ) {
      if (this.text[this.index] !== '"') throw new CanonicalError("invalid_json");
      const key = this.string();
      if (keys.has(key)) throw new CanonicalError("duplicate_key");
      keys.add(key);
      this.bumpEntry();
      this.space();
      if (this.text[this.index] !== ":") throw new CanonicalError("invalid_json");
      this.index += 1;
      this.space();
      values[key] = this.value(depth);
      this.space();
      const separator = this.text[this.index];
      this.index += 1;
      if (separator === "}") return values;
      if (separator !== ",") throw new CanonicalError("invalid_json");
      this.space();
    }
  }
  bumpEntry() {
    this.entries += 1;
    if (this.entries > this.limits.maxEntries) throw new CanonicalError("too_many_entries");
  }
  space() {
    for (; ; ) {
      const character = this.text[this.index];
      if (character !== " " && character !== "	" && character !== "\n" && character !== "\r") {
        return;
      }
      this.index += 1;
    }
  }
};
function hasLoneSurrogate(value) {
  for (let index = 0; index < value.length; index += 1) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit >= 55296 && codeUnit <= 56319) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 56320 && next <= 57343)) return true;
      index += 1;
    } else if (codeUnit >= 56320 && codeUnit <= 57343) {
      return true;
    }
  }
  return false;
}
function parseCanonicalJson(text, limits = DEFAULT_CANONICAL_LIMITS) {
  return new Parser(text, limits).parse();
}
function isJsonArray(value) {
  return Array.isArray(value);
}
function isJsonObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function canonicalize(value) {
  if (value === null || typeof value === "boolean") return String(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new CanonicalError("invalid_number");
    const encoded = JSON.stringify(Object.is(value, -0) ? 0 : value);
    return encoded;
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (isJsonArray(value)) return `[${value.map(canonicalize).join(",")}]`;
  if (isJsonObject(value)) {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalize(value[key] ?? null)}`).join(",")}}`;
  }
  throw new CanonicalError("serialization_failed");
}

// src/signals/name.ts
var SIGNAL_NAME_PATTERN = /^[a-z][a-z0-9._-]{0,63}$/u;
function isSignalName(value) {
  return typeof value === "string" && SIGNAL_NAME_PATTERN.test(value);
}

// src/async-updates/envelope.ts
var MAX_U64 = (1n << 64n) - 1n;
var MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;
var OPERATION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
var SIGNAL_SCOPE = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/u;
var SUBSCRIPTION_ID = /^[A-Za-z0-9_-]{16,128}$/u;
var ASYNC_LIMITS = Object.freeze({
  maxBytes: 64 * 1024,
  maxDepth: 8,
  maxEntries: 1024,
  maxStringBytes: 4096
});
var MAX_PAYLOAD_BYTES = 32 * 1024;
var AsyncEnvelopeError = class extends Error {
  constructor(code) {
    super(code);
    this.code = code;
    this.name = "AsyncEnvelopeError";
  }
};
function fail(code) {
  throw new AsyncEnvelopeError(code);
}
function record(value, code = "async_envelope_invalid") {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(code);
  return value;
}
function exact(value, keys, code = "async_envelope_invalid") {
  const present = Object.keys(value);
  if (present.length !== keys.length || keys.some((key) => !Object.prototype.hasOwnProperty.call(value, key))) {
    fail(code);
  }
}
function string(value, code = "async_envelope_invalid") {
  if (typeof value !== "string") fail(code);
  return value;
}
function integer(value, code = "async_envelope_invalid") {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) fail(code);
  return value;
}
function counter(value) {
  const encoded = string(value);
  if (!/^(?:0|[1-9][0-9]*)$/u.test(encoded)) fail("async_position_invalid");
  const parsed = BigInt(encoded);
  if (parsed > MAX_U64) fail("async_position_invalid");
  return parsed;
}
function freezeJson(value) {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) {
    const values = value;
    return Object.freeze(values.map((item) => freezeJson(item)));
  }
  const source = value;
  const frozen = /* @__PURE__ */ Object.create(null);
  for (const key of Object.keys(source)) frozen[key] = freezeJson(source[key] ?? null);
  return Object.freeze(frozen);
}
function schemaMatches(schema, value) {
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
function presentationSignalSchemaMatches(schema, value) {
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
function targetValid(target) {
  return target === "self" || target === "parent" || target === "child" || target === "document" || /^named_island:[a-z][a-z0-9._-]{0,63}$/u.test(target) || /^browser:[a-z][a-z0-9._-]{0,63}$/u.test(target);
}
function payload(value, membership2) {
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
        "async_payload_invalid"
      );
      const name = string(fields["event"], "async_payload_unregistered");
      const schemaVersion = integer(fields["schema_version"], "async_payload_unregistered");
      const target = string(fields["target"], "async_payload_unregistered");
      const event = membership2.events.find((candidate) => candidate.name === name);
      const eventPayload = fields["payload"] ?? null;
      if (event === void 0 || !OPERATION_NAME.test(name) || !Number.isSafeInteger(event.maximumFanout) || event.maximumFanout < 1 || event.maximumFanout > 256 || event.version !== schemaVersion || !targetValid(target) || !event.targets.includes(target) || !schemaMatches(event.schema, eventPayload)) {
        fail("async_payload_unregistered");
      }
      return Object.freeze({
        event: name,
        kind: "browser_event",
        payload: freezeJson(eventPayload),
        schema_version: schemaVersion,
        target
      });
    }
    case "presentation_signal": {
      exact(fields, ["kind", "name", "scope", "value"], "async_payload_invalid");
      const name = string(fields["name"], "async_payload_unregistered");
      const scope = string(fields["scope"], "async_payload_unregistered");
      const contract = membership2.presentationSignals.find(
        (candidate) => candidate.name === name && candidate.scope === scope
      );
      const signalValue = fields["value"] ?? null;
      if (contract === void 0 || !isSignalName(name) || !SIGNAL_SCOPE.test(scope) || !presentationSignalSchemaMatches(contract.schema, signalValue)) {
        fail("async_payload_unregistered");
      }
      return Object.freeze({
        kind: "presentation_signal",
        name,
        scope,
        value: freezeJson(signalValue)
      });
    }
    case "heartbeat":
      exact(fields, ["kind"], "async_payload_invalid");
      return Object.freeze({ kind: "heartbeat" });
    case "complete": {
      exact(fields, ["kind", "reason"], "async_payload_invalid");
      const reason = string(fields["reason"], "async_payload_invalid");
      if (reason !== "server_shutdown" && reason !== "subscription_retired" && reason !== "stream_completed") {
        fail("async_payload_invalid");
      }
      return Object.freeze({ kind: "complete", reason });
    }
    case "error": {
      exact(fields, ["code", "kind"], "async_payload_invalid");
      const code = string(fields["code"], "async_payload_invalid");
      if (code !== "authorization_lost" && code !== "replay_unavailable" && code !== "backpressure" && code !== "stream_unavailable") {
        fail("async_payload_invalid");
      }
      return Object.freeze({ code, kind: "error" });
    }
    default:
      fail("async_payload_unsupported");
  }
}
function decodeAsyncEnvelope(encoded, membership2) {
  let parsed;
  try {
    parsed = parseCanonicalJson(encoded, ASYNC_LIMITS);
  } catch (error) {
    if (error instanceof CanonicalError && error.code === "duplicate_key") {
      fail("duplicate_async_envelope_field");
    }
    fail("async_envelope_invalid");
  }
  if (canonicalize(parsed) !== encoded) fail("async_envelope_noncanonical");
  const fields = record(parsed);
  exact(fields, ["payload", "position", "protocol_version", "stream", "subscription"]);
  if (fields["protocol_version"] !== 1) fail("async_protocol_unsupported");
  const subscriptionId = string(fields["subscription"]);
  if (!SUBSCRIPTION_ID.test(subscriptionId) || subscriptionId !== membership2.subscriptionId) {
    fail("async_subscription_mismatch");
  }
  const stream = string(fields["stream"]);
  if (!OPERATION_NAME.test(stream) || stream !== membership2.stream) fail("async_stream_mismatch");
  const positionFields = record(fields["position"] ?? null, "async_position_invalid");
  exact(positionFields, ["epoch", "sequence"], "async_position_invalid");
  const position2 = Object.freeze({
    epoch: counter(positionFields["epoch"]),
    sequence: counter(positionFields["sequence"])
  });
  return Object.freeze({
    payload: payload(fields["payload"] ?? null, membership2),
    position: position2,
    protocolVersion: 1,
    stream,
    subscriptionId
  });
}
function comparePosition(left, right) {
  if (left.epoch !== right.epoch) return left.epoch < right.epoch ? -1 : 1;
  if (left.sequence === right.sequence) return 0;
  return left.sequence < right.sequence ? -1 : 1;
}
function isExactSuccessor(current, candidate) {
  return candidate.epoch === current.epoch && current.sequence < MAX_U64 && candidate.sequence === current.sequence + 1n;
}

// src/async-updates/continuity.ts
function copy(position2) {
  return Object.freeze({ epoch: position2.epoch, sequence: position2.sequence });
}
var ContinuityMachine = class {
  #position;
  #state = "disconnected";
  #requiredHighWater = null;
  #nonReplayableHighWater = null;
  #proofRequired = false;
  constructor(baseline) {
    this.#position = copy(baseline);
  }
  state() {
    return this.#state;
  }
  position() {
    return copy(this.#position);
  }
  connected() {
    if (this.#state !== "closed") this.#state = "connecting";
  }
  transportLost() {
    if (this.#state !== "closed") {
      this.#proofRequired = true;
      this.#state = "reconnecting";
    }
  }
  degrade() {
    if (this.#state !== "closed") {
      this.#proofRequired = true;
      this.#state = "degraded";
    }
  }
  degradeAt(candidate) {
    this.#recordHighWater(candidate);
    this.degrade();
  }
  degradeNonReplayableAt(candidate) {
    this.#recordHighWater(candidate);
    if (this.#nonReplayableHighWater === null || comparePosition(candidate, this.#nonReplayableHighWater) > 0) {
      this.#nonReplayableHighWater = copy(candidate);
    }
    this.degrade();
  }
  close() {
    this.#state = "closed";
  }
  observe(candidate) {
    if (this.#state === "closed") return "closed";
    const ordering = comparePosition(candidate, this.#position);
    if (ordering === 0) return "duplicate";
    if (ordering < 0 || candidate.epoch < this.#position.epoch) return "stale";
    if (this.#proofRequired) {
      this.#recordHighWater(candidate);
      return "continuity_required";
    }
    if (isExactSuccessor(this.#position, candidate)) return "apply";
    this.#recordHighWater(candidate);
    this.#state = "degraded";
    return "gap";
  }
  commit(candidate) {
    if (!isExactSuccessor(this.#position, candidate))
      throw new Error("async_sequence_commit_invalid");
    this.#position = copy(candidate);
    this.#state = "current";
  }
  validateReplay(positions) {
    if (this.#state === "closed" || positions.length === 0 || positions.length > 1024) {
      throw new Error("async_replay_invalid");
    }
    let prior = this.#position;
    for (const position2 of positions) {
      if (!isExactSuccessor(prior, position2)) throw new Error("async_replay_invalid");
      if (this.#nonReplayableHighWater !== null && comparePosition(position2, this.#nonReplayableHighWater) >= 0) {
        throw new Error("async_replay_non_replayable");
      }
      prior = position2;
    }
    if (this.#requiredHighWater !== null && comparePosition(prior, this.#requiredHighWater) < 0) {
      throw new Error("async_replay_incomplete");
    }
  }
  finishReplay() {
    if (this.#state === "closed") throw new Error("async_replay_invalid");
    this.#requiredHighWater = null;
    this.#proofRequired = false;
    this.#state = "current";
  }
  proveAuthoritativeBaseline(position2) {
    this.validateAuthoritativeBaseline(position2);
    this.#position = copy(position2);
    this.#requiredHighWater = null;
    this.#nonReplayableHighWater = null;
    this.#proofRequired = false;
    this.#state = "current";
  }
  validateAuthoritativeBaseline(position2) {
    const validPosition = this.#nonReplayableHighWater === null ? comparePosition(position2, this.#position) === 0 : comparePosition(position2, this.#nonReplayableHighWater) >= 0;
    if (this.#state === "closed" || !validPosition || this.#requiredHighWater !== null && comparePosition(position2, this.#requiredHighWater) < 0) {
      throw new Error("async_replay_incomplete");
    }
  }
  acceptsAuthoritativeBaseline(position2) {
    try {
      this.validateAuthoritativeBaseline(position2);
      return true;
    } catch {
      return false;
    }
  }
  #recordHighWater(candidate) {
    if (this.#requiredHighWater === null || comparePosition(candidate, this.#requiredHighWater) > 0) {
      this.#requiredHighWater = copy(candidate);
    }
  }
};

// src/conformance.ts
import { readFile } from "node:fs/promises";
var FIXTURE_FILES_V1 = [
  "canonical-success.json",
  "canonical-failure.json",
  "snapshot-success.json",
  "snapshot-failure.json",
  "protocol-success.json",
  "protocol-failure.json",
  "response-ordering.json",
  "compatibility.json"
];
var FIXTURE_FILES_V2 = [
  "protocol-success.json",
  "protocol-failure.json",
  "compatibility.json"
];
var FIXTURE_FILES_V3 = [
  "compatibility.json",
  "diagnostics.json",
  "directive-grammar.json",
  "island-metadata.json",
  "morph-identity.json",
  "navigation.json",
  "response-application.json",
  "runtime-config.json",
  "scheduling.json"
];
var FIXTURE_FILES_V4 = Object.freeze([
  "async-envelope.json",
  "compatibility.json",
  "diagnostics.json",
  "directive-grammar.json",
  "resource-lifecycle.json",
  "runtime-features.json",
  "upload-protocol.json"
]);
var FIXTURE_SETS = [
  { version: 1, files: FIXTURE_FILES_V1 },
  { version: 2, files: FIXTURE_FILES_V2 },
  { version: 3, files: FIXTURE_FILES_V3 },
  { version: 4, files: FIXTURE_FILES_V4 }
];
function fixtureSet(version2) {
  const fixtureSet2 = FIXTURE_SETS.find((candidate) => candidate.version === version2);
  if (fixtureSet2 === void 0) throw new TypeError("unsupported_fixture_version");
  return fixtureSet2;
}
function fixtureDirectory(version2) {
  return new URL(`../../fixtures/v${String(version2)}/`, import.meta.url);
}
async function loadFixtureSet(version2 = 1) {
  const fixture = fixtureSet(version2);
  const directory = fixtureDirectory(version2);
  const entries = await Promise.all(
    fixture.files.map(async (name) => {
      const text = await readFile(new URL(name, directory), "utf8");
      const value = JSON.parse(text);
      return [name, value];
    })
  );
  return new Map(entries);
}

// src/features/bounded.ts
var HARD_MAX_RESOURCE_ITEMS = 65536;
var HARD_MAX_RESOURCE_BYTES = 1024 * 1024 * 1024;
var HARD_MAX_ACTIVE_PERMITS = 65536;
var CALLBACK_READ_FAILED = /* @__PURE__ */ Symbol("bounded_owner_callback_read_failed");
function validLimit(value, maximum) {
  return Number.isSafeInteger(value) && value >= 1 && value <= maximum;
}
function validItemBytes(value) {
  return Number.isSafeInteger(value) && value >= 0;
}
function isNullish(value) {
  return value === null || value === void 0;
}
function isCallback(value) {
  return typeof value === "function";
}
function readLifecycleCallback(resource, property) {
  try {
    const callback2 = resource[property];
    return typeof callback2 === "function" ? () => {
      Reflect.apply(callback2, resource, []);
    } : callback2;
  } catch {
    return CALLBACK_READ_FAILED;
  }
}
var BoundedOwner = class {
  #limits;
  #queue = [];
  #queueHead = 0;
  #queuedItems = 0;
  #leases = /* @__PURE__ */ new Set();
  #waiters = /* @__PURE__ */ new Set();
  #waiterBatch = null;
  #resources = /* @__PURE__ */ new Set();
  #pendingResources = /* @__PURE__ */ new Set();
  #deferredResources = /* @__PURE__ */ new Set();
  #state = "active";
  #queuedBytes = 0;
  #active = 0;
  #waitingPermits = 0;
  #ownedResources = 0;
  #canceled = false;
  #pumping = false;
  #transitioning = false;
  #notifyingRegistration = false;
  #advancingPending = false;
  #resourceCallbackDepth = 0;
  #resourceValidationDepth = 0;
  #validationTrackAllowance = 0;
  constructor(limits) {
    const maxItems = limits.maxItems;
    const maxBytes = limits.maxBytes;
    const maxActive = limits.maxActive;
    if (!validLimit(maxItems, HARD_MAX_RESOURCE_ITEMS) || !validLimit(maxBytes, HARD_MAX_RESOURCE_BYTES) || !validLimit(maxActive, HARD_MAX_ACTIVE_PERMITS)) {
      throw new RangeError("bounded_owner_limits");
    }
    this.#limits = Object.freeze({ maxActive, maxBytes, maxItems });
  }
  enqueue(value, bytes) {
    if (this.#state === "retired") return "retired";
    if (isNullish(value)) throw new TypeError("bounded_owner_item_value");
    if (!validItemBytes(bytes)) throw new RangeError("bounded_owner_item_bytes");
    if (this.#queuedItems >= this.#limits.maxItems) return "items_exceeded";
    if (bytes > this.#limits.maxBytes - this.#queuedBytes) return "bytes_exceeded";
    this.#queue.push({ bytes, value });
    this.#queuedItems += 1;
    this.#queuedBytes += bytes;
    return "accepted";
  }
  dequeue() {
    if (this.#transitioning || this.#state !== "active") return null;
    const item = this.#queue[this.#queueHead];
    if (item === void 0) return null;
    this.#queue[this.#queueHead] = void 0;
    this.#queueHead += 1;
    this.#queuedItems -= 1;
    this.#queuedBytes -= item.bytes;
    this.#compactQueue();
    return item.value;
  }
  acquire() {
    this.#pumpWaiters();
    if (this.#state !== "active" || this.#transitioning || this.#active >= this.#limits.maxActive || this.#waitingPermits > 0) {
      return null;
    }
    return this.#createLease();
  }
  requestPermit(admit) {
    if (typeof admit !== "function") throw new TypeError("bounded_owner_permit_callback");
    this.#pumpWaiters();
    const priorWaitersRemain = this.#waitingPermits > 0;
    const waiter = {
      admit,
      lease: null,
      state: this.#state === "retired" ? "retired" : this.#waitingPermits >= this.#limits.maxItems ? "items_exceeded" : "waiting"
    };
    const request = Object.freeze({
      dispose: () => {
        this.#cancelRequest(waiter);
      },
      state: () => waiter.state
    });
    if (waiter.state !== "waiting") return request;
    this.#waiters.add(waiter);
    this.#waitingPermits += 1;
    if (!priorWaitersRemain) this.#pumpWaiters();
    return request;
  }
  cancel() {
    if (this.#canceled) return false;
    this.#canceled = true;
    return true;
  }
  isCanceled() {
    return this.#canceled;
  }
  track(resource) {
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    if (this.#state === "active" && this.#resourceValidationDepth === 0 && this.#resourceCallbackDepth === 0 && !this.#transitioning && !this.#notifyingRegistration && !this.#advancingPending && !this.#pumping) {
      this.#advancePendingResources();
    }
    if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    if (this.#resourceValidationDepth > 0) {
      if (this.#validationTrackAllowance < 1) {
        throw new Error("bounded_owner_resource_reentrant");
      }
      this.#validationTrackAllowance -= 1;
    }
    if (this.#ownedResources >= this.#limits.maxItems) {
      throw new Error("bounded_owner_resource_limit");
    }
    let dispose;
    let resume;
    let suspend;
    this.#resourceValidationDepth += 1;
    try {
      dispose = readLifecycleCallback(resource, "dispose");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
      resume = readLifecycleCallback(resource, "resume");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
      suspend = readLifecycleCallback(resource, "suspend");
      if (this.#inState("retired")) throw new Error("bounded_owner_retired");
    } finally {
      this.#resourceValidationDepth -= 1;
    }
    if (dispose === CALLBACK_READ_FAILED || resume === CALLBACK_READ_FAILED || suspend === CALLBACK_READ_FAILED || !isCallback(dispose) || resume !== void 0 && !isCallback(resume) || suspend !== void 0 && !isCallback(suspend)) {
      throw new TypeError("bounded_owner_resource");
    }
    if (this.#ownedResources >= this.#limits.maxItems) {
      throw new Error("bounded_owner_resource_limit");
    }
    const edgeState = this.#state;
    const record2 = { activated: false, active: true, dispose, resume, suspend };
    this.#resources.add(record2);
    this.#pendingResources.add(record2);
    this.#ownedResources += 1;
    if (this.#transitioning || this.#notifyingRegistration || this.#advancingPending || this.#pumping || this.#resourceCallbackDepth > 0) {
      this.#deferredResources.add(record2);
      return Object.freeze({
        dispose: () => {
          this.#disposeResource(record2);
        }
      });
    }
    this.#notifyingRegistration = true;
    try {
      if (edgeState === "active") this.#activateResource(record2);
    } finally {
      this.#notifyingRegistration = false;
    }
    return Object.freeze({
      dispose: () => {
        this.#disposeResource(record2);
      }
    });
  }
  suspend() {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "active") return this.#state;
    this.#deferredResources.clear();
    this.#state = "suspended";
    this.#transitioning = true;
    const resources2 = [...this.#resources];
    try {
      for (let index = resources2.length - 1; index >= 0; index -= 1) {
        const record2 = resources2[index];
        if (record2?.active === true && record2.activated) {
          this.#invokeResourceCallback(record2.suspend);
        }
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("suspended");
    } finally {
      this.#transitioning = false;
    }
    return this.#state;
  }
  resume() {
    if (this.#transitioning) return this.#state;
    if (this.#state !== "suspended") return this.#state;
    this.#deferredResources.clear();
    this.#transitioning = true;
    const resources2 = [...this.#resources];
    try {
      for (const record2 of resources2) {
        if (record2.active) this.#activateResource(record2);
        if (!this.#inState("suspended")) break;
      }
      this.#drainDeferredResources("active");
    } finally {
      this.#transitioning = false;
    }
    if (this.#inState("suspended")) this.#state = "active";
    this.#pumpWaiters();
    return this.#state;
  }
  retire() {
    if (this.#state === "retired") {
      return Object.freeze({ drainedBytes: 0, drainedItems: 0, releasedPermits: 0 });
    }
    this.#state = "retired";
    this.cancel();
    const drainedItems = this.#queuedItems;
    const drainedBytes = this.#queuedBytes;
    const releasedPermits = this.#active;
    this.#queue = [];
    this.#queueHead = 0;
    this.#queuedItems = 0;
    this.#queuedBytes = 0;
    for (const waiter of this.#waiters) {
      if (waiter.state === "waiting") waiter.state = "retired";
    }
    if (this.#waiterBatch !== null) {
      for (const waiter of this.#waiterBatch) {
        if (waiter.state === "waiting") waiter.state = "retired";
      }
    }
    this.#waiters.clear();
    this.#waiterBatch?.clear();
    this.#waiterBatch = null;
    this.#waitingPermits = 0;
    for (const lease of this.#leases) lease.active = false;
    this.#leases.clear();
    this.#active = 0;
    const resources2 = [...this.#resources].filter((record2) => record2.active).reverse();
    this.#resources.clear();
    this.#pendingResources.clear();
    this.#deferredResources.clear();
    for (const record2 of resources2) {
      record2.active = false;
      this.#ownedResources -= 1;
    }
    for (const record2 of resources2) this.#invokeResourceCallback(record2.dispose);
    return Object.freeze({ drainedBytes, drainedItems, releasedPermits });
  }
  snapshot() {
    return Object.freeze({
      active: this.#active,
      canceled: this.#canceled,
      ownedResources: this.#ownedResources,
      pendingResources: this.#pendingResources.size,
      queuedBytes: this.#queuedBytes,
      queuedItems: this.#queuedItems,
      state: this.#state,
      waitingPermits: this.#waitingPermits
    });
  }
  #createLease() {
    const record2 = { active: true };
    this.#leases.add(record2);
    this.#active += 1;
    return Object.freeze({
      dispose: () => {
        this.#release(record2);
      }
    });
  }
  #release(record2) {
    if (!record2.active) return;
    record2.active = false;
    this.#leases.delete(record2);
    this.#active -= 1;
    this.#pumpWaiters();
  }
  #cancelRequest(waiter) {
    if (waiter.state === "waiting") {
      waiter.state = "canceled";
      this.#waitingPermits -= 1;
      this.#waiters.delete(waiter);
      this.#waiterBatch?.delete(waiter);
      this.#pumpWaiters();
      return;
    }
    if (waiter.state !== "admitted") return;
    waiter.state = "canceled";
    waiter.lease?.dispose();
  }
  #pumpWaiters() {
    if (this.#pumping || this.#state !== "active") return;
    this.#pumping = true;
    const eligible = this.#waiters;
    this.#waiters = /* @__PURE__ */ new Set();
    this.#waiterBatch = eligible;
    try {
      for (const waiter of eligible) {
        if (this.#active >= this.#limits.maxActive) break;
        eligible.delete(waiter);
        if (waiter.state !== "waiting") continue;
        this.#waitingPermits -= 1;
        const lease = this.#createLease();
        waiter.lease = lease;
        waiter.state = "admitted";
        try {
          waiter.admit(lease);
        } catch {
          this.#cancelRequest(waiter);
        }
        if (!this.#admissionOpen()) break;
      }
    } finally {
      if (!this.#inState("retired")) {
        const additions = this.#waiters;
        for (const waiter of additions) eligible.add(waiter);
        this.#waiters = eligible;
      }
      this.#waiterBatch = null;
      this.#pumping = false;
    }
  }
  #activateResource(record2) {
    if (!record2.active) return;
    record2.activated = true;
    this.#pendingResources.delete(record2);
    this.#deferredResources.delete(record2);
    this.#invokeResourceCallback(record2.resume);
  }
  #disposeResource(record2) {
    if (!record2.active) return;
    record2.active = false;
    this.#resources.delete(record2);
    this.#pendingResources.delete(record2);
    this.#deferredResources.delete(record2);
    this.#ownedResources -= 1;
    this.#invokeResourceCallback(record2.dispose);
  }
  #advancePendingResources() {
    if (this.#advancingPending || this.#state !== "active") return;
    this.#advancingPending = true;
    const eligible = [...this.#pendingResources];
    try {
      for (const record2 of eligible) {
        if (!this.#inState("active")) break;
        if (record2.active && this.#pendingResources.has(record2)) this.#activateResource(record2);
      }
    } finally {
      this.#advancingPending = false;
    }
  }
  #invokeResourceCallback(callback2) {
    const priorAllowance = this.#validationTrackAllowance;
    this.#resourceCallbackDepth += 1;
    if (this.#resourceValidationDepth > 0) this.#validationTrackAllowance = 1;
    try {
      callback2?.();
    } catch {
    } finally {
      this.#validationTrackAllowance = priorAllowance;
      this.#resourceCallbackDepth -= 1;
    }
  }
  #compactQueue() {
    if (this.#queuedItems === 0) {
      this.#queue = [];
      this.#queueHead = 0;
      return;
    }
    if (this.#queueHead < 1024 || this.#queueHead * 2 < this.#queue.length) return;
    this.#queue = this.#queue.slice(this.#queueHead);
    this.#queueHead = 0;
  }
  #admissionOpen() {
    return !this.#transitioning && this.#state === "active";
  }
  #inState(state) {
    return this.#state === state;
  }
  #drainDeferredResources(target) {
    const eligible = [...this.#deferredResources];
    this.#deferredResources.clear();
    for (const record2 of eligible) {
      if (record2.active && this.#state !== "retired") {
        if (target === "active") this.#activateResource(record2);
        else if (record2.activated) this.#invokeResourceCallback(record2.suspend);
      }
    }
  }
};

// src/directives/parser.ts
var MAX_PRESENT_DIRECTIVES = 64;

// src/islands/metadata.ts
var ISLAND_ROOT_SELECTOR = "[data-suprnova-live-island]";
var ISLAND_STATUS_ATTRIBUTE = "data-suprnova-live-status";
var REQUIRED_ATTRIBUTES = [
  "data-suprnova-live-component",
  "data-suprnova-live-contract",
  "data-suprnova-live-document-key",
  "data-suprnova-live-lazy-complete",
  "data-suprnova-live-protocol-min",
  "data-suprnova-live-revision",
  "data-suprnova-live-root",
  "data-suprnova-live-slot",
  "data-suprnova-live-snapshot",
  "data-suprnova-live-snapshot-kind"
];
var KNOWN_ATTRIBUTES = /* @__PURE__ */ new Set([
  "data-suprnova-live-island",
  "data-suprnova-live-instance",
  ISLAND_STATUS_ATTRIBUTE,
  ...REQUIRED_ATTRIBUTES
]);

// src/features/host.ts
var RUNTIME_FEATURE_DRIVER_FORMAT = /* @__PURE__ */ Symbol.for("suprnova.live.feature-driver.v1");
var RUNTIME_FEATURE_DRIVER_CORE_RANGE = 1099511758848;

// src/features/contract.ts
var VALIDATED_ASYNC_DESCRIPTORS = /* @__PURE__ */ new WeakMap();
function consumeValidatedAsyncDescriptor(owner, capability) {
  const token = capability;
  const record2 = VALIDATED_ASYNC_DESCRIPTORS.get(token);
  if (record2?.owner !== owner) {
    throw new Error("async_descriptor_capability_invalid");
  }
  VALIDATED_ASYNC_DESCRIPTORS.delete(token);
  return record2.authorization;
}
var RUNTIME_FEATURE_FORMAT = /* @__PURE__ */ Symbol.for("suprnova.live.feature.v1");
var RUNTIME_FEATURE_CORE_RANGE = RUNTIME_FEATURE_DRIVER_CORE_RANGE;
var RUNTIME_STIMULUS_ADAPTER_FORMAT = /* @__PURE__ */ Symbol.for(
  "suprnova.live.feature.stimulus-adapter.v1"
);
var MAXIMUM_DISPOSERS = 64;
var MAXIMUM_DRIVER_ISLANDS = 256;
var MAXIMUM_SCANNED_ELEMENTS = 4096;
var MAXIMUM_FEATURE_DIRECTIVES = 2048;
var UPLOADS = /* @__PURE__ */ new WeakMap();
var ASYNC = /* @__PURE__ */ new WeakMap();
function callback(owner, property, required) {
  let value;
  try {
    value = Reflect.get(owner, property);
  } catch {
    throw new TypeError("feature_controller_invalid");
  }
  if (value === void 0 && !required) return null;
  if (typeof value !== "function") throw new TypeError("feature_controller_invalid");
  return (...arguments_) => Reflect.apply(value, owner, arguments_);
}
function normalizeStimulusBridge(input) {
  if (typeof input !== "object" && typeof input !== "function" || input === null) {
    throw new TypeError("feature_controller_invalid");
  }
  const dispose = callback(input, "dispose", true);
  if (dispose === null) throw new TypeError("feature_controller_invalid");
  try {
    const beforeMorph = callback(input, "beforeMorph", true);
    const afterMorph = callback(input, "afterMorph", true);
    const disposeScope = callback(input, "disposeScope", true);
    if (beforeMorph === null || afterMorph === null || disposeScope === null) {
      throw new TypeError("feature_controller_invalid");
    }
    return Object.freeze({
      afterMorph: (continuity, scope) => {
        afterMorph(continuity, scope);
      },
      beforeMorph: (scope) => beforeMorph(scope),
      dispose: () => {
        dispose();
      },
      disposeScope: (scope) => {
        disposeScope(scope);
      }
    });
  } catch (error) {
    invoke(() => {
      dispose();
    });
    throw error;
  }
}
function normalizeController(input) {
  if (typeof input !== "object" && typeof input !== "function" || input === null) {
    throw new TypeError("feature_controller_invalid");
  }
  const dispose = callback(input, "dispose", true);
  if (dispose === null) throw new TypeError("feature_controller_invalid");
  try {
    const resume = callback(input, "resume", false);
    const suspend = callback(input, "suspend", false);
    return Object.freeze([
      () => {
        dispose();
      },
      resume === null ? null : () => {
        resume();
      },
      suspend === null ? null : () => {
        suspend();
      }
    ]);
  } catch (error) {
    invoke(() => {
      dispose();
    });
    throw error;
  }
}
function normalizeIslandController(input) {
  const base = normalizeController(input);
  try {
    const beforeMorph = callback(input, "beforeMorph", false);
    const afterMorph = callback(input, "afterMorph", false);
    const abortMorph = callback(input, "abortMorph", false);
    return Object.freeze([...base, beforeMorph, afterMorph, abortMorph]);
  } catch (error) {
    invoke(base[0]);
    throw error;
  }
}
function normalizeDocumentController(input) {
  const base = normalizeController(input);
  try {
    const connectIsland = callback(input, "connectIsland", true);
    if (connectIsland === null) throw new TypeError("feature_controller_invalid");
    return Object.freeze([
      ...base,
      (port) => connectIsland(port)
    ]);
  } catch (error) {
    invoke(base[0]);
    throw error;
  }
}
function invoke(callback2) {
  try {
    callback2?.();
    return true;
  } catch {
    return false;
  }
}
function own(disposers, dispose) {
  if (typeof dispose !== "function" || disposers.length >= MAXIMUM_DISPOSERS) {
    throw new TypeError("feature_disposer_invalid");
  }
  disposers.push(dispose);
}
function disposeOwnership(ownership) {
  let clean = true;
  for (let index = ownership[1].length - 1; index >= 0; index -= 1) {
    clean = invoke(ownership[1][index] ?? null) && clean;
  }
  ownership[1].length = 0;
  return invoke(ownership[0]?.[0] ?? null) && clean;
}
function* featureElements(root, node = root) {
  if (node !== root && node.matches(ISLAND_ROOT_SELECTOR)) return;
  yield node;
  for (const child of node.children) yield* featureElements(root, child);
  const shadow = "shadowRoot" in node ? node.shadowRoot : null;
  if (shadow !== null) for (const child of shadow.children) yield* featureElements(root, child);
}
function queryFeatureDirectiveOwnership(root, parser, capability, diagnose) {
  if (typeof parser !== "function") return Object.freeze([]);
  const found = [];
  let scanned = 0;
  try {
    for (const element of featureElements(root)) {
      scanned += 1;
      if (scanned > MAXIMUM_SCANNED_ELEMENTS) break;
      const attributes = [];
      let inspectedAttributes = 0;
      for (const attribute of element.attributes) {
        inspectedAttributes += 1;
        if (inspectedAttributes > MAX_PRESENT_DIRECTIVES) {
          diagnose("resource_exhausted");
          return Object.freeze([]);
        }
        const name = attribute.name;
        if (name.startsWith("live:")) attributes.push({ name, value: attribute.value });
      }
      const names = Object.freeze(attributes.map(({ name }) => name));
      for (const attribute of attributes) {
        if (found.length >= MAXIMUM_FEATURE_DIRECTIVES) return Object.freeze(found);
        const directive = parser(attribute.name, attribute.value, names);
        if (directive.ok && directive.capability === capability) {
          found.push(Object.freeze({ attributeName: attribute.name, directive, element }));
        }
      }
    }
  } catch {
    return Object.freeze([]);
  }
  return Object.freeze(found);
}
function defineFeature(slot, definition, cache) {
  if (typeof definition !== "object" && typeof definition !== "function" || definition === null) {
    throw new TypeError("feature_definition_invalid");
  }
  const cached = cache.get(definition);
  if (cached !== void 0) return cached;
  const connectDocument = callback(definition, "connectDocument", true);
  if (connectDocument === null) throw new TypeError("feature_definition_invalid");
  const identity = Object.freeze({});
  let document = null;
  const documentDisposers = [];
  const islands = /* @__PURE__ */ new Map();
  let documentContext = null;
  let retired = false;
  let connected = false;
  const isRetired = () => retired;
  const drive = (event, value) => {
    if (event === 0) {
      if (retired || connected || value === null || !("diagnose" in value)) return false;
      connected = true;
      const port = value;
      const track = port.trackResource?.bind(port);
      const context = Object.freeze({
        diagnose: (detail) => {
          port.diagnose(detail);
        },
        onDispose: (dispose) => {
          own(documentDisposers, dispose);
        },
        ...track === void 0 ? {} : {
          trackResource: (kind, dispose) => track.call(port, kind, dispose)
        }
      });
      let connectedDocument;
      try {
        connectedDocument = normalizeDocumentController(connectDocument(context));
      } catch (error) {
        disposeOwnership([null, documentDisposers]);
        throw error;
      }
      if (isRetired()) {
        for (let index = documentDisposers.length - 1; index >= 0; index -= 1) {
          invoke(documentDisposers[index] ?? null);
        }
        documentDisposers.length = 0;
        invoke(connectedDocument[0]);
        return false;
      }
      documentContext = context;
      document = connectedDocument;
      return true;
    }
    if (event === 1) {
      const activeDocumentContext = documentContext;
      if (retired || document === null || activeDocumentContext === null || value === null || !("element" in value)) {
        return false;
      }
      const port = value;
      const ownsIsland = () => islands.has(port.element);
      if (islands.has(port.element)) return true;
      const disposers = [];
      let controller = null;
      const pending = [null, disposers];
      islands.set(port.element, pending);
      const sharedPort = {
        element: port.element,
        identity: port.identity,
        onDispose: (dispose) => {
          own(disposers, dispose);
        },
        queryDirectiveOwnership: (parser) => queryFeatureDirectiveOwnership(
          port.element,
          parser,
          slot === 0 ? "uploads@1" : "async@1",
          (detail) => {
            activeDocumentContext.diagnose(detail);
          }
        )
      };
      const featurePort = slot === 0 ? Object.freeze({
        ...sharedPort,
        proposeUploadHandle: (field, proposal) => port.proposeUploadHandle(field, proposal)
      }) : Object.freeze({
        ...sharedPort,
        captureAsyncStatusBaseline: () => port.captureAsyncStatusBaseline?.(),
        clearAsyncStatus: () => port.clearAsyncStatus?.(),
        consumeRegisteredEventCapability: (descriptor) => {
          const authorization = consumeValidatedAsyncDescriptor(
            featurePort,
            descriptor
          );
          return port.authorizeRegisteredEvents(
            Object.freeze({
              descriptorBinding: authorization.descriptorBinding,
              events: authorization.events
            })
          );
        },
        dispatchRegisteredEvent: (capability, event2) => port.dispatchRegisteredEvent(capability, event2),
        enqueueFreshRender: (reason, completion, completionKey) => {
          if (completion === void 0) return port.enqueueFreshRender(reason);
          return completionKey === void 0 ? port.enqueueFreshRender(reason, completion) : port.enqueueFreshRender(reason, completion, completionKey);
        },
        projectAsyncStatus: (state) => port.projectAsyncStatus?.(state),
        writePresentationSignal: (scope, name, signalValue) => port.writePresentationSignal(scope, name, signalValue)
      });
      try {
        const connectedIsland = document[3](featurePort);
        if (connectedIsland !== void 0) controller = normalizeIslandController(connectedIsland);
      } catch (error) {
        if (islands.get(port.element) === pending) islands.delete(port.element);
        disposeOwnership([controller, disposers]);
        throw error;
      }
      if (isRetired() || !ownsIsland()) {
        disposeOwnership([controller, disposers]);
        return false;
      }
      islands.set(port.element, [controller, disposers]);
      return true;
    }
    if (event === 4) {
      if (value === null || !("nodeType" in value)) return false;
      const ownership = islands.get(value);
      if (ownership === void 0) return true;
      islands.delete(value);
      return disposeOwnership(ownership);
    }
    if (event === 6 || event === 7 || event === 8) {
      if (value === null || !("nodeType" in value)) return false;
      const controller = islands.get(value)?.[0];
      if (controller === void 0 || controller === null) return true;
      return invoke(controller[event === 6 ? 3 : event === 7 ? 4 : 5]);
    }
    if (event === 2 || event === 3) {
      let clean2 = true;
      const controllers = [...islands.values()];
      if (event === 2) {
        for (let index = controllers.length - 1; index >= 0; index -= 1) {
          clean2 = invoke(controllers[index]?.[0]?.[2] ?? null) && clean2;
        }
        clean2 = invoke(document?.[2] ?? null) && clean2;
      } else {
        clean2 = invoke(document?.[1] ?? null) && clean2;
        for (const ownership of controllers) clean2 = invoke(ownership[0]?.[1] ?? null) && clean2;
      }
      return clean2;
    }
    if (retired) return false;
    retired = true;
    documentContext = null;
    let clean = true;
    const ownerships = [...islands.values()];
    islands.clear();
    for (let index = ownerships.length - 1; index >= 0; index -= 1) {
      const ownership = ownerships[index];
      if (ownership !== void 0) clean = disposeOwnership(ownership) && clean;
    }
    for (let index = documentDisposers.length - 1; index >= 0; index -= 1) {
      clean = invoke(documentDisposers[index] ?? null) && clean;
    }
    documentDisposers.length = 0;
    return invoke(document?.[0] ?? null) && clean;
  };
  const feature = Object.freeze([
    RUNTIME_FEATURE_FORMAT,
    slot,
    1,
    RUNTIME_FEATURE_CORE_RANGE,
    identity,
    drive
  ]);
  cache.set(definition, feature);
  return feature;
}
function defineUploadsFeature(definition) {
  return defineFeature(0, definition, UPLOADS);
}
function defineAsyncFeature(definition) {
  return defineFeature(1, definition, ASYNC);
}
function inspectRuntimeFeature(input) {
  if (!Array.isArray(input) || !Object.isFrozen(input) || input.length !== 6) return null;
  try {
    if (Reflect.ownKeys(input).length !== 7) return null;
    const values = [];
    for (let index = 0; index < 6; index += 1) {
      const descriptor = Object.getOwnPropertyDescriptor(input, index);
      if (descriptor === void 0 || !("value" in descriptor)) return null;
      values.push(descriptor.value);
    }
    const slot = values[1];
    const identity = values[4];
    if (values[0] !== RUNTIME_FEATURE_FORMAT || slot !== 0 && slot !== 1 || values[2] !== 1 || values[3] !== RUNTIME_FEATURE_CORE_RANGE || typeof identity !== "object" && typeof identity !== "function" || identity === null || !Object.isFrozen(identity) || Reflect.ownKeys(identity).length !== 0 || typeof values[5] !== "function") {
      return null;
    }
    return [input, slot, values[5]];
  } catch {
    return null;
  }
}
function inspectStimulusAdapter(input) {
  if (!Array.isArray(input) || !Object.isFrozen(input) || input.length !== 5) return null;
  try {
    if (Reflect.ownKeys(input).length !== 6) return null;
    const descriptors = Object.getOwnPropertyDescriptors(input);
    const values = [];
    for (let index = 0; index < 5; index += 1) values.push(descriptors[index]?.value);
    if (values[0] !== RUNTIME_STIMULUS_ADAPTER_FORMAT || values[1] !== 1 || values[2] !== RUNTIME_FEATURE_CORE_RANGE || typeof values[3] !== "symbol" || typeof values[4] !== "function") {
      return null;
    }
    return input;
  } catch {
    return null;
  }
}
function createOptionalFeatureDriver() {
  const entries = [null, null];
  const islands = /* @__PURE__ */ new Map();
  const stimulusContinuities = /* @__PURE__ */ new Map();
  let documentPort = null;
  let ready = 0;
  let size = 0;
  let started = 0;
  let state = 0;
  let stimulusAdapter = null;
  let stimulus = null;
  const isActive = () => state === 1;
  const report2 = (detail) => {
    try {
      documentPort?.diagnose(detail);
    } catch {
    }
  };
  const connectStimulus = () => {
    const options = documentPort?.stimulus;
    if (stimulus !== null || options === void 0) return true;
    const adapter = stimulusAdapter;
    if (adapter === null) {
      report2("contract_mismatch");
      return false;
    }
    const diagnostics2 = {
      record(input) {
        report2(
          input.detailCode === "resource_exhausted" ? "resource_exhausted" : "operation_rejected"
        );
      }
    };
    try {
      const connected = normalizeStimulusBridge(
        Reflect.apply(adapter[4], adapter, [options, diagnostics2])
      );
      if (state !== 1 || stimulusAdapter !== adapter || documentPort?.stimulus !== options) {
        connected.dispose();
        return false;
      }
      stimulus = connected;
      return true;
    } catch {
      report2("operation_rejected");
      return false;
    }
  };
  const run = (entry, event, value) => {
    try {
      const completed = Reflect.apply(entry[2], entry[0], [event, value]);
      if (completed === true) return true;
    } catch {
    }
    report2("operation_rejected");
    return false;
  };
  const connect = (entry, island) => {
    const bit = 1 << entry[1];
    if (state !== 1 || (ready & bit) === 0 || (island[1] & bit) !== 0) return;
    island[1] |= bit;
    run(entry, 1, island[0]);
  };
  const start = (entry) => {
    const bit = 1 << entry[1];
    if (state !== 1 || (started & bit) !== 0 || documentPort === null) return;
    started |= bit;
    const track = documentPort.trackResource?.bind(documentPort);
    const context = Object.freeze({
      diagnose: report2,
      onDispose(dispose) {
        if (typeof dispose !== "function") report2("operation_rejected");
      },
      ...track === void 0 ? {} : {
        trackResource: (kind, dispose) => track.call(documentPort, kind, dispose)
      }
    });
    if (!run(entry, 0, context) || entries[entry[1]] !== entry) return;
    ready |= bit;
    for (const island of islands.values()) connect(entry, island);
  };
  const drive = (event, value) => {
    if (event === 0) {
      if (state !== 0 || value === null || !("diagnose" in value)) return false;
      state = 1;
      documentPort = value;
      connectStimulus();
      for (const entry of [...entries]) if (entry !== null) start(entry);
      return true;
    }
    if (event === 1) {
      if (state !== 1 || value === null || !("element" in value)) return false;
      if (islands.has(value.element)) return true;
      if (islands.size >= MAXIMUM_DRIVER_ISLANDS) {
        report2("resource_exhausted");
        return false;
      }
      const island = [value, 0];
      islands.set(value.element, island);
      for (const entry of [...entries]) if (entry !== null) connect(entry, island);
      return true;
    }
    if (event === 6 || event === 7 || event === 8) {
      if (state === 3 || value === null || !("nodeType" in value)) return false;
      const island = islands.get(value);
      if (island !== void 0) {
        for (let slot = 0; slot <= 1; slot += 1) {
          const entry = entries[slot];
          if (entry !== void 0 && entry !== null && (island[1] & 1 << slot) !== 0) {
            run(entry, event, value);
          }
        }
      }
      const bridge2 = stimulus;
      if (bridge2 === null) return true;
      if (event === 6) {
        const previous = stimulusContinuities.get(value);
        stimulusContinuities.delete(value);
        if (previous !== void 0) {
          try {
            bridge2.disposeScope(value);
          } catch {
            report2("operation_rejected");
          }
        }
        let continuity2;
        try {
          continuity2 = bridge2.beforeMorph(value);
        } catch {
          report2("operation_rejected");
          return true;
        }
        if (state !== 1 || stimulus !== bridge2 || !islands.has(value)) return false;
        stimulusContinuities.set(value, continuity2);
        return true;
      }
      const continuity = stimulusContinuities.get(value);
      stimulusContinuities.delete(value);
      if (continuity !== void 0) {
        try {
          if (event === 7) bridge2.afterMorph(continuity, value);
          else bridge2.disposeScope(value);
        } catch {
          report2("operation_rejected");
        }
      }
      return true;
    }
    if (event === 4) {
      if (state === 3 || value === null || !("nodeType" in value)) return false;
      stimulusContinuities.delete(value);
      const island = islands.get(value);
      if (island === void 0) return true;
      islands.delete(value);
      try {
        stimulus?.disposeScope(value);
      } catch {
        report2("operation_rejected");
      }
      for (let slot = 1; slot >= 0; slot -= 1) {
        const entry = entries[slot];
        if (entry !== void 0 && entry !== null && (island[1] & 1 << slot) !== 0) {
          run(entry, 4, value);
        }
      }
      return true;
    }
    if (event === 2) {
      if (state !== 1) return false;
      state = 2;
      const scopes = [...stimulusContinuities.keys()];
      stimulusContinuities.clear();
      for (const scope of scopes) {
        try {
          stimulus?.disposeScope(scope);
        } catch {
          report2("operation_rejected");
        }
      }
      for (let slot = 1; slot >= 0; slot -= 1) {
        const entry = entries[slot];
        if (entry !== void 0 && entry !== null && (ready & 1 << slot) !== 0) {
          run(entry, 2, null);
        }
      }
      return true;
    }
    if (event === 3) {
      if (state !== 2) return false;
      state = 1;
      connectStimulus();
      if (!isActive()) return false;
      for (const entry of [...entries]) {
        if (entry === null) continue;
        if ((started & 1 << entry[1]) === 0) start(entry);
        else if ((ready & 1 << entry[1]) !== 0) run(entry, 3, null);
      }
      return true;
    }
    if (state === 3) return false;
    state = 3;
    const owned = [...entries];
    const claimed = started;
    const bridge = stimulus;
    const diagnosticPort = documentPort;
    entries[0] = null;
    entries[1] = null;
    islands.clear();
    stimulusContinuities.clear();
    stimulus = null;
    stimulusAdapter = null;
    documentPort = null;
    ready = 0;
    size = 0;
    started = 0;
    for (let slot = 1; slot >= 0; slot -= 1) {
      const entry = owned[slot];
      if (entry !== void 0 && entry !== null && (claimed & 1 << slot) !== 0) {
        run(entry, 5, null);
      }
    }
    try {
      bridge?.dispose();
    } catch {
      try {
        diagnosticPort?.diagnose("operation_rejected");
      } catch {
      }
    }
    return true;
  };
  const driver = Object.freeze([
    RUNTIME_FEATURE_DRIVER_FORMAT,
    1,
    RUNTIME_FEATURE_DRIVER_CORE_RANGE,
    Object.freeze({}),
    drive
  ]);
  return Object.freeze({
    driver,
    register(feature) {
      if (state === 3) return "incompatible";
      const entry = inspectRuntimeFeature(feature);
      if (entry === null) return "incompatible";
      const current = entries[entry[1]];
      if (current !== null) return current[0] === feature ? "already_registered" : "conflict";
      if (size >= 2) return "registry_full";
      entries[entry[1]] = entry;
      size += 1;
      if (state === 1) start(entry);
      return "registered";
    },
    registerStimulus(adapter) {
      if (state === 3) return "incompatible";
      const inspected = inspectStimulusAdapter(adapter);
      if (inspected === null) return "incompatible";
      if (stimulusAdapter !== null) {
        return stimulusAdapter[3] === adapter[3] ? "already_registered" : "conflict";
      }
      stimulusAdapter = inspected;
      if (state === 1) connectStimulus();
      return "registered";
    }
  });
}

// src/runtime/limits.ts
var RUNTIME_CONFIG_LIMITS = Object.freeze({
  maxBytes: 16384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 2048,
  minRequestTimeoutMs: 100,
  maxRequestTimeoutMs: 12e4,
  minResponseBytes: 1024,
  maxResponseBytes: 4194304,
  maxQueuedPerIsland: 64,
  maxParallelPerIsland: 8,
  maxAllowedOrigins: 32,
  maxAssetIdentityUnits: 128
});
var MAX_DIAGNOSTIC_ENTRIES = 1024;
var MAX_DIAGNOSTIC_SEQUENCE = 4294967295;
function boundedInteger(value, minimum, maximum) {
  return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum;
}

// src/runtime/diagnostics.ts
var DIAGNOSTIC_CODES = [
  "configuration_invalid",
  "runtime_duplicate",
  "island_invalid",
  "directive_invalid",
  "scheduler_rejected",
  "transport_failed",
  "response_invalid",
  "morph_failed",
  "effect_failed",
  "navigation_failed",
  "lifecycle_notice",
  "resource_limit"
];
var DIAGNOSTIC_SEVERITIES = ["error", "warning", "info"];
var DIAGNOSTIC_PHASES = [
  "configuration",
  "discovery",
  "directive",
  "schedule",
  "transport",
  "response",
  "morph",
  "effect",
  "navigation",
  "lifecycle"
];
var DIAGNOSTIC_DETAILS = [
  "missing_element",
  "duplicate_element",
  "invalid_shape",
  "unsupported_version",
  "unsafe_endpoint",
  "origin_not_allowed",
  "resource_exhausted",
  "contract_mismatch",
  "operation_rejected",
  "network_failure",
  "invalid_response",
  "recovery_required",
  "handler_missing",
  "connected",
  "disconnected"
];
function contains(values, candidate) {
  return typeof candidate === "string" && values.some((value) => value === candidate);
}
function validInput(input) {
  const candidate = input;
  return candidate !== null && typeof candidate === "object" && contains(DIAGNOSTIC_CODES, candidate.code) && contains(DIAGNOSTIC_SEVERITIES, candidate.severity) && contains(DIAGNOSTIC_PHASES, candidate.phase) && contains(DIAGNOSTIC_DETAILS, candidate.detailCode);
}
var RuntimeDiagnostics = class {
  #mode;
  #maximum;
  #emit;
  #entries = [];
  #sequence;
  constructor(options) {
    const maximum = options.maxEntries ?? 256;
    const sequence = options.initialSequence ?? 0;
    if (!boundedInteger(maximum, 1, MAX_DIAGNOSTIC_ENTRIES)) {
      throw new RangeError("runtime_diagnostic_limit");
    }
    if (!boundedInteger(sequence, 0, MAX_DIAGNOSTIC_SEQUENCE)) {
      throw new RangeError("runtime_diagnostic_sequence");
    }
    if (!["off", "errors", "verbose"].some((mode) => mode === options.mode)) {
      throw new RangeError("runtime_diagnostic_mode");
    }
    this.#mode = options.mode;
    this.#maximum = maximum;
    this.#sequence = sequence;
    this.#emit = options.emit;
  }
  record(input, unsafeContext) {
    void unsafeContext;
    if (!validInput(input) || this.#mode === "off" || this.#mode === "errors" && input.severity !== "error" || this.#entries.length >= this.#maximum || this.#sequence > MAX_DIAGNOSTIC_SEQUENCE) {
      return null;
    }
    const diagnostic = Object.freeze({
      code: input.code,
      severity: input.severity,
      phase: input.phase,
      detailCode: input.detailCode,
      sequence: this.#sequence
    });
    this.#entries.push(diagnostic);
    try {
      this.#emit?.(diagnostic);
    } catch {
    }
    this.#sequence += 1;
    return diagnostic;
  }
  entries() {
    return Object.freeze([...this.#entries]);
  }
};

// src/uploads/state.ts
var UploadProtocolStateError = class extends Error {
  constructor(code) {
    super(code);
    this.code = code;
    this.name = "UploadProtocolStateError";
  }
};
var MAX_U642 = 18446744073709551615n;
var MAX_RETAINED_OUTCOMES = 64;
function parseUploadProtocolState(value) {
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
      throw new UploadProtocolStateError("invalid_upload_transition");
  }
}
function parseUploadProtocolTransition(value) {
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
      throw new UploadProtocolStateError("invalid_upload_transition");
  }
}
function isTerminalUploadProtocolState(state) {
  return state === "canceled" || state === "expired" || state === "failed" || state === "finalized" || state === "rejected";
}
function nextState(state, transition) {
  if (isTerminalUploadProtocolState(state)) {
    throw new UploadProtocolStateError("invalid_upload_transition");
  }
  switch (transition) {
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
      if (state === "created" || state === "queued" || state === "ready" || state === "transferring" || state === "verifying") {
        return "canceled";
      }
      break;
    case "reject":
      if (state === "verifying") return "rejected";
      break;
    case "expire":
      if (state === "created" || state === "queued" || state === "ready" || state === "transferring" || state === "verifying") {
        return "expired";
      }
      break;
    case "fail":
      return "failed";
    default:
      return assertNever(transition);
  }
  throw new UploadProtocolStateError("invalid_upload_transition");
}
function assertNever(value) {
  void value;
  throw new UploadProtocolStateError("invalid_upload_transition");
}
var UploadProtocolStateMachine = class {
  #outcomes = /* @__PURE__ */ new Map();
  #revision;
  #state;
  constructor(state, revision) {
    this.#state = parseUploadProtocolState(state);
    if (revision < 0n || revision > MAX_U642) {
      throw new UploadProtocolStateError("revision_exhausted");
    }
    this.#revision = revision;
  }
  get state() {
    return this.#state;
  }
  get revision() {
    return this.#revision;
  }
  apply(request) {
    const transition = parseUploadProtocolTransition(request.transition);
    const existing = this.#outcomes.get(request.idempotencyKey);
    if (existing !== void 0) {
      if (existing.expectedRevision !== request.expectedRevision || existing.transition !== transition) {
        throw new UploadProtocolStateError("upload_conflict");
      }
      return Object.freeze({ ...existing.outcome, disposition: "existing_outcome" });
    }
    if (request.expectedRevision !== this.#revision) {
      throw new UploadProtocolStateError("upload_conflict");
    }
    if (this.#outcomes.size === MAX_RETAINED_OUTCOMES) {
      throw new UploadProtocolStateError("upload_idempotency_history_full");
    }
    if (this.#revision === MAX_U642) {
      throw new UploadProtocolStateError("revision_exhausted");
    }
    const state = nextState(this.#state, transition);
    const outcome = Object.freeze({
      disposition: "applied",
      revision: this.#revision + 1n,
      state
    });
    this.#outcomes.set(
      request.idempotencyKey,
      Object.freeze({
        expectedRevision: request.expectedRevision,
        outcome,
        transition
      })
    );
    this.#revision = outcome.revision;
    this.#state = state;
    return outcome;
  }
};

// src/uploads/types.ts
var DEFAULT_UPLOAD_CHUNK_BYTES = 256 * 1024;
var MAX_UPLOAD_CHUNK_BYTES = 4 * 1024 * 1024;
var MAX_UPLOAD_QUEUE_BYTES = 4 * 1024 * 1024;
var UPLOAD_FIELD = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/u;
var UPLOAD_HANDLE = /^[0-9a-f]{8}-[0-9a-f]{4}-[47][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u;
var IDEMPOTENCY_KEY = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/u;
var SHA256 = /^[0-9a-f]{64}$/u;
var REVISION = /^(?:0|[1-9][0-9]{0,19})$/u;
var MAX_U643 = 18446744073709551615n;
var RESPONSE_STATES = Object.freeze({
  cancel: ["canceled", "expired", "finalized"],
  complete: ["verifying", "ready", "failed", "canceled", "expired"],
  create: ["queued"],
  put_chunk: ["transferring", "verifying", "ready", "failed", "canceled", "expired"],
  status: [
    "queued",
    "transferring",
    "verifying",
    "ready",
    "finalizing",
    "finalized",
    "failed",
    "canceled",
    "expired"
  ]
});
function validateUploadField(field) {
  if (typeof field !== "string" || !UPLOAD_FIELD.test(field)) {
    throw new Error("upload_field_invalid");
  }
}
function validateUploadHandle(handle) {
  if (typeof handle !== "string" || !UPLOAD_HANDLE.test(handle)) {
    throw new Error("upload_handle_invalid");
  }
}
function validateUploadIdempotencyKey(value) {
  if (typeof value !== "string" || !IDEMPOTENCY_KEY.test(value)) {
    throw new Error("upload_idempotency_key_invalid");
  }
}
function validateUploadChecksum(value) {
  if (typeof value !== "string" || !SHA256.test(value)) {
    throw new Error("upload_checksum_invalid");
  }
}
function validateUploadRevision(value) {
  if (typeof value !== "string" || !REVISION.test(value) || BigInt(value) > MAX_U643) {
    throw new Error("upload_revision_invalid");
  }
}

// src/uploads/protocol.ts
var UPLOAD_LIMITS = Object.freeze({
  maxBytes: 16384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 4096
});
var MAX_U32 = 4294967295;
var UploadProtocolError = class extends Error {
  constructor(code) {
    super(code);
    this.code = code;
    this.name = "UploadProtocolError";
  }
};
function fail2(code) {
  throw new UploadProtocolError(code);
}
function object(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail2("invalid_field");
  return value;
}
function operation(value) {
  switch (value) {
    case "cancel":
    case "complete":
    case "create":
    case "put_chunk":
    case "reacquire":
    case "status":
      return value;
    default:
      fail2("unsupported_operation");
  }
}
function exact2(fields, expected) {
  const present = Object.keys(fields);
  if (present.length !== expected.length || expected.some((key) => !Object.prototype.hasOwnProperty.call(fields, key))) {
    fail2("unknown_field");
  }
}
function operationKeys(value) {
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
        "size"
      ];
    case "complete":
      return [
        "expected_revision",
        "handle",
        "idempotency_key",
        "operation",
        "protocol_version",
        "whole_checksum"
      ];
    case "cancel":
      return ["expected_revision", "handle", "idempotency_key", "operation", "protocol_version"];
    case "reacquire":
    case "status":
      return ["handle", "operation", "protocol_version"];
  }
}
function validateFields(operationName, fields) {
  try {
    switch (operationName) {
      case "create":
        validateUploadRevision(fields["expected_revision"]);
        if (fields["expected_revision"] !== "0") fail2("invalid_field");
        validateUploadField(fields["field"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        break;
      case "put_chunk":
        validateUploadHandle(fields["handle"]);
        validateUploadRevision(fields["expected_revision"]);
        validateUploadIdempotencyKey(fields["idempotency_key"]);
        if (typeof fields["chunk_index"] !== "number" || !Number.isSafeInteger(fields["chunk_index"]) || fields["chunk_index"] < 0 || fields["chunk_index"] > MAX_U32 || typeof fields["size"] !== "number" || !Number.isSafeInteger(fields["size"]) || fields["size"] < 1) {
          fail2("invalid_field");
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
  } catch (error) {
    if (error instanceof UploadProtocolError) throw error;
    fail2("invalid_field");
  }
}
function decodeUploadProtocolOperation(encoded) {
  let parsed;
  try {
    parsed = parseCanonicalJson(encoded, UPLOAD_LIMITS);
  } catch (error) {
    if (error instanceof CanonicalError && error.code === "duplicate_key") {
      fail2("duplicate_field");
    }
    fail2("invalid_field");
  }
  if (canonicalize(parsed) !== encoded) fail2("noncanonical");
  const fields = object(parsed);
  if (fields["protocol_version"] !== 1) fail2("unsupported_protocol");
  const operationName = operation(fields["operation"]);
  exact2(fields, operationKeys(operationName));
  validateFields(operationName, fields);
  return Object.freeze({ operation: operationName });
}

// tests/support/iteration-004-conformance.ts
var SUBSCRIPTION = "c3Vic2NyaXB0aW9uLTAwMQ";
var UPLOAD_LIMITS2 = Object.freeze({
  maxBytes: 16384,
  maxDepth: 8,
  maxEntries: 64,
  maxStringBytes: 4096
});
function asObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("invalid_object");
  }
  return value;
}
function assertFixtureOracle(actual, expected, path) {
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
      assertFixtureOracle(actual[field], value, `${path}.${field}`);
    }
    return;
  }
  if (!Object.is(actual, expected)) {
    throw new Error(`fixture_oracle_value_mismatch:${path}`);
  }
}
function parseUploadOperation(encoded) {
  try {
    decodeUploadProtocolOperation(encoded);
    return { code: null, disposition: "accepted" };
  } catch (error) {
    return {
      code: error instanceof UploadProtocolError ? error.code : "invalid_field",
      disposition: "rejected"
    };
  }
}
function transitionCase(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("invalid_transition_case");
  }
  const fields = value;
  if (typeof fields["id"] !== "string" || typeof fields["expected_revision"] !== "string" || typeof fields["next_revision"] !== "string" || fields["expected"] !== "applied" && fields["expected"] !== "conflict" && fields["expected"] !== "existing_outcome" || fields["current_revision"] !== void 0 && typeof fields["current_revision"] !== "string" || fields["idempotency_key"] !== void 0 && typeof fields["idempotency_key"] !== "string") {
    throw new Error("invalid_transition_case");
  }
  return Object.freeze({
    currentRevision: typeof fields["current_revision"] === "string" ? BigInt(fields["current_revision"]) : null,
    expectedDisposition: fields["expected"],
    expectedRevision: BigInt(fields["expected_revision"]),
    id: fields["id"],
    idempotencyKey: typeof fields["idempotency_key"] === "string" ? fields["idempotency_key"] : fields["id"],
    nextRevision: BigInt(fields["next_revision"]),
    operation: parseUploadProtocolTransition(fields["operation"]),
    retry: fields["retry"] !== void 0,
    state: parseUploadProtocolState(fields["from"]),
    to: parseUploadProtocolState(fields["to"])
  });
}
function runUploadTransition(fixture) {
  const request = Object.freeze({
    expectedRevision: fixture.expectedRevision,
    idempotencyKey: fixture.idempotencyKey,
    transition: fixture.operation
  });
  let machine = new UploadProtocolStateMachine(
    fixture.state,
    fixture.currentRevision ?? fixture.expectedRevision
  );
  let outcome;
  try {
    if (fixture.retry) {
      machine = new UploadProtocolStateMachine(fixture.state, fixture.expectedRevision);
      machine.apply(request);
    }
    outcome = machine.apply(request);
  } catch (error) {
    const code = error instanceof UploadProtocolStateError ? error.code : "invalid_upload_transition";
    outcome = {
      code,
      disposition: code === "upload_conflict" ? "conflict" : "rejected"
    };
  }
  return {
    code: "code" in outcome ? outcome.code : null,
    disposition: outcome.disposition,
    id: fixture.id,
    position: String(machine.revision),
    state: machine.state
  };
}
function position(value) {
  return Object.freeze({ epoch: BigInt(value.epoch), sequence: BigInt(value.sequence) });
}
function encodedPosition(value) {
  return Object.freeze({ epoch: String(value.epoch), sequence: String(value.sequence) });
}
function membership(signalName = "completion_percent") {
  return Object.freeze({
    authorization: Object.freeze({ kind: "session_cookie" }),
    baseline: Object.freeze({ epoch: 4n, sequence: 40n }),
    descriptorBinding: "descriptor-conformance-001",
    document: Object.freeze({
      authorizationScope: "document-conformance",
      origin: "https://app.example.test",
      transport: "sse"
    }),
    events: Object.freeze([
      Object.freeze({
        cycle: Object.freeze({ kind: "forbid_repeated_island" }),
        maximumFanout: 1,
        name: "orders.updated",
        order: "per_source_sequence",
        payloadContract: "orders.updated.payload",
        schema: "json",
        source: "stream",
        targets: Object.freeze(["self"]),
        version: 1
      })
    ]),
    expiresAt: 1e4,
    fallbackPoll: Object.freeze({
      initial: "wait",
      intervalMs: 3e4,
      jitterRatio: 0.2,
      visibility: "visible"
    }),
    heartbeatTimeoutMs: 3e4,
    presentationSignals: Object.freeze([
      Object.freeze({ name: signalName, scope: "root-scope", schema: "u64" })
    ]),
    reconnect: Object.freeze({
      kind: "resume_or_refresh",
      maximumAttempts: 4,
      maximumDelayMs: 3e4,
      minimumDelayMs: 250
    }),
    stream: "orders",
    subscriptionId: SUBSCRIPTION
  });
}
function asyncCode(error) {
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
function runContinuityCase(fixture) {
  const machine = new ContinuityMachine(position(fixture.baseline));
  machine.proveAuthoritativeBaseline(position(fixture.baseline));
  let disposition;
  if (fixture.observed !== void 0) {
    const observed = position(fixture.observed);
    const result = machine.observe(observed);
    if (result === "apply") {
      machine.commit(observed);
      disposition = "apply";
    } else if (result === "duplicate") disposition = "ignore_duplicate";
    else if (result === "gap") disposition = "degrade";
    else throw new Error(`unmapped_continuity_observation:${result}`);
  } else {
    if (fixture.observed_gap === void 0 || fixture.recovery === void 0) {
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
    state: machine.state()
  };
}
var EXPECTED_V4_INVENTORY = Object.freeze({
  "async-envelope.json": [
    "codec_limits",
    "continuity_cases",
    "envelope_cases",
    "live_protocol_versions",
    "payload_kinds",
    "protocol_versions",
    "schema_version",
    "signal_name_cases",
    "subscription_states"
  ],
  "compatibility.json": [
    "cases",
    "compatible_core",
    "live_protocol_versions",
    "schema_version",
    "snapshot_versions"
  ],
  "diagnostics.json": [
    "allowed_dimensions",
    "codes",
    "phases",
    "redacted_classes",
    "redaction_cases",
    "retention",
    "schema_version",
    "severities"
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
    "transition_modifiers"
  ],
  "resource-lifecycle.json": ["bounds", "cases", "resource_kinds", "schema_version", "states"],
  "runtime-features.json": [
    "allowed_island_operations",
    "features",
    "forbidden_island_operations",
    "registration_outcomes",
    "registry",
    "retirement",
    "schema_version"
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
    "transition_cases"
  ]
});
function fixtureRecord(fixtures2, name) {
  const value = fixtures2.get(name);
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`invalid_v4_fixture:${name}`);
  }
  return value;
}
function fixtureArray(record2, name) {
  const value = record2[name];
  if (!Array.isArray(value)) throw new Error(`invalid_v4_collection:${name}`);
  return value;
}
function v4Inventory(fixtures2) {
  const names = [...fixtures2.keys()].sort();
  if (JSON.stringify(names) !== JSON.stringify([...FIXTURE_FILES_V4].sort())) {
    throw new Error("v4_fixture_file_inventory_changed");
  }
  const inventory2 = Object.fromEntries(
    names.map((name) => [name, Object.keys(fixtureRecord(fixtures2, name)).sort()])
  );
  if (JSON.stringify(inventory2) !== JSON.stringify(EXPECTED_V4_INVENTORY)) {
    throw new Error("v4_fixture_collection_inventory_changed");
  }
  return inventory2;
}
function runSignalNameCase(fixture, presentationTemplate2) {
  const template = asObject(parseCanonicalJson(presentationTemplate2.encoded, UPLOAD_LIMITS2));
  const payload2 = asObject(template["payload"]);
  const candidate = Object.freeze({
    ...template,
    payload: Object.freeze({ ...payload2, name: fixture.value })
  });
  try {
    decodeAsyncEnvelope(canonicalize(candidate), membership(fixture.value));
    return { code: null, disposition: "accepted", id: fixture.value };
  } catch {
    return { code: "invalid_signal_name", disposition: "rejected", id: fixture.value };
  }
}
function version(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)$/u.exec(value);
  if (match === null) throw new Error("invalid_fixture_semver");
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}
function compareVersion(left, right) {
  const leftParts = version(left);
  const rightParts = version(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    const delta = (leftParts[index] ?? 0) - (rightParts[index] ?? 0);
    if (delta !== 0) return delta;
  }
  return 0;
}
function actualFeature(name) {
  const definition = {
    connectDocument() {
      return { connectIsland: () => void 0, dispose: () => void 0 };
    }
  };
  if (name === "uploads") return defineUploadsFeature(definition);
  if (name === "async") return defineAsyncFeature(definition);
  return null;
}
function runCompatibilityCase(fixture, contracts) {
  if (!fixture.present) {
    return { code: null, disposition: "ordinary_live_available", id: fixture.id };
  }
  const contract = contracts.find((candidate2) => candidate2.name === fixture.feature);
  const feature = actualFeature(fixture.feature);
  if (contract === void 0 || feature === null) {
    return { code: "feature_unavailable", disposition: "feature_unavailable", id: fixture.id };
  }
  const coreCompatible = compareVersion(fixture.core_version, contract.compatible_core.minimum) >= 0 && compareVersion(fixture.core_version, contract.compatible_core.maximum_exclusive) < 0;
  const candidate = [...feature];
  candidate[2] = fixture.capability_version;
  candidate[3] = coreCompatible ? RUNTIME_FEATURE_CORE_RANGE : 0;
  const registration = createOptionalFeatureDriver().register(
    Object.freeze(candidate)
  );
  const compatible = fixture.capability_version === contract.capability_version && registration === "registered";
  return {
    code: compatible ? null : "feature_unavailable",
    disposition: compatible ? "compatible" : "feature_unavailable",
    id: fixture.id
  };
}
function runRedactionCase(fixture) {
  const diagnostics2 = new RuntimeDiagnostics({ maxEntries: 1, mode: "verbose" });
  diagnostics2.record(
    {
      code: "configuration_invalid",
      detailCode: "invalid_shape",
      phase: "configuration",
      severity: "error"
    },
    fixture.sample
  );
  const serialized = JSON.stringify(diagnostics2.entries());
  const unsafe = typeof fixture.sample === "string" ? fixture.sample : JSON.stringify(fixture.sample);
  const redacted = !serialized.includes(unsafe);
  return {
    code: redacted ? null : "diagnostic_value_leaked",
    disposition: redacted ? "redacted" : "rejected",
    id: fixture.id,
    state: redacted ? "[redacted]" : null
  };
}
function runResourceCase(fixture, bounds) {
  const owner = new BoundedOwner({
    maxActive: bounds.max_active,
    maxBytes: bounds.max_bytes,
    maxItems: bounds.max_items
  });
  let lease = null;
  return fixture.operations.map((value, index) => {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("invalid_resource_operation");
    }
    const operation2 = value;
    let outcome;
    switch (operation2["operation"]) {
      case "enqueue": {
        if (typeof operation2["bytes"] !== "number") throw new Error("invalid_resource_bytes");
        outcome = owner.enqueue(`item-${String(index)}`, operation2["bytes"]);
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
          released_permits: retirement.releasedPermits
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
      state: owner.snapshot().state
    };
  });
}
var fixtures = await loadFixtureSet(4);
var inventory = v4Inventory(fixtures);
var upload = fixtureRecord(fixtures, "upload-protocol.json");
var asynchronous = fixtureRecord(fixtures, "async-envelope.json");
var compatibility = fixtureRecord(fixtures, "compatibility.json");
var diagnostics = fixtureRecord(fixtures, "diagnostics.json");
var resources = fixtureRecord(fixtures, "resource-lifecycle.json");
var runtimeFeatures = fixtureRecord(fixtures, "runtime-features.json");
fixtureRecord(fixtures, "directive-grammar.json");
var codecCases = fixtureArray(upload, "codec_cases");
var transitionCases = fixtureArray(upload, "transition_cases").map(transitionCase);
var envelopeCases = fixtureArray(asynchronous, "envelope_cases");
var continuityCases = fixtureArray(asynchronous, "continuity_cases");
var signalCases = fixtureArray(asynchronous, "signal_name_cases");
var compatibilityCases = fixtureArray(compatibility, "cases");
var redactionCases = fixtureArray(diagnostics, "redaction_cases");
var resourceCases = fixtureArray(resources, "cases");
var presentationTemplate = envelopeCases.find((fixture) => fixture.id === "presentation-signal");
if (presentationTemplate === void 0) throw new Error("missing_presentation_signal_template");
var rejectedUnknownTransition = false;
try {
  parseUploadProtocolTransition("future_internal_transition");
} catch {
  rejectedUnknownTransition = true;
}
if (!rejectedUnknownTransition) throw new Error("unknown_internal_transition_was_accepted");
var wireOperations = [
  "create",
  "put_chunk",
  "status",
  "complete",
  "cancel",
  "reacquire"
];
if (JSON.stringify(upload["operations"]) !== JSON.stringify(wireOperations)) {
  throw new Error("upload_wire_operations_changed");
}
var report = {
  async_continuity: continuityCases.map(runContinuityCase),
  async_envelopes: envelopeCases.map((fixture) => {
    try {
      const decoded = decodeAsyncEnvelope(fixture.encoded, membership());
      return {
        code: null,
        disposition: "accepted",
        id: fixture.id,
        position: encodedPosition(decoded.position)
      };
    } catch (error) {
      return { code: asyncCode(error), disposition: "rejected", id: fixture.id, position: null };
    }
  }),
  async_signals: signalCases.map((fixture) => runSignalNameCase(fixture, presentationTemplate)),
  compatibility: compatibilityCases.map(
    (fixture) => runCompatibilityCase(
      fixture,
      fixtureArray(runtimeFeatures, "features")
    )
  ),
  diagnostics: redactionCases.map(runRedactionCase),
  inventory,
  resource_lifecycle: resourceCases.flatMap(
    (fixture) => runResourceCase(fixture, resources["bounds"])
  ),
  upload_codecs: codecCases.map((fixture) => ({
    id: fixture.id,
    ...parseUploadOperation(fixture.encoded)
  })),
  upload_transitions: transitionCases.map(runUploadTransition)
};
for (const [index, fixture] of codecCases.entries()) {
  assertFixtureOracle(
    report.upload_codecs[index],
    {
      code: fixture.expected === "accepted" ? null : fixture.expected,
      disposition: fixture.expected === "accepted" ? "accepted" : "rejected",
      id: fixture.id
    },
    `upload_codecs.${fixture.id}`
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
      state: fixture.to
    },
    `upload_transitions.${fixture.id}`
  );
}
for (const [index, fixture] of envelopeCases.entries()) {
  assertFixtureOracle(
    report.async_envelopes[index],
    {
      code: fixture.expected === "accepted" ? null : fixture.expected,
      disposition: fixture.expected === "accepted" ? "accepted" : "rejected",
      id: fixture.id
    },
    `async_envelopes.${fixture.id}`
  );
}
for (const [index, fixture] of signalCases.entries()) {
  assertFixtureOracle(
    report.async_signals[index],
    {
      code: fixture.expected === "accepted" ? null : "invalid_signal_name",
      disposition: fixture.expected,
      id: fixture.value
    },
    `async_signals.${fixture.value}`
  );
}
for (const [index, fixture] of continuityCases.entries()) {
  assertFixtureOracle(
    report.async_continuity[index],
    { disposition: fixture.expected, id: fixture.id, state: fixture.state },
    `async_continuity.${fixture.id}`
  );
}
for (const [index, fixture] of compatibilityCases.entries()) {
  assertFixtureOracle(
    report.compatibility[index],
    {
      code: fixture.expected === "feature_unavailable" ? "feature_unavailable" : null,
      disposition: fixture.expected,
      id: fixture.id
    },
    `compatibility.${fixture.id}`
  );
}
for (const [index, fixture] of redactionCases.entries()) {
  assertFixtureOracle(
    report.diagnostics[index],
    { code: null, disposition: "redacted", id: fixture.id, state: fixture.expected },
    `diagnostics.${fixture.id}`
  );
}
var resourceIndex = 0;
for (const fixture of resourceCases) {
  for (const [operationIndex, value] of fixture.operations.entries()) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new Error("invalid_resource_operation_oracle");
    }
    const expected = value["expected"];
    assertFixtureOracle(
      report.resource_lifecycle[resourceIndex],
      { id: `${fixture.id}:${String(operationIndex)}`, outcome: expected },
      `resource_lifecycle.${fixture.id}.${String(operationIndex)}`
    );
    resourceIndex += 1;
  }
}
process.stdout.write(`${JSON.stringify(report)}
`);
