import { canonicalize, type JsonValue } from "../canonical.js";
import type { AsyncPayloadSchema, AsyncRegisteredEventContract } from "../async-updates/types.js";
import type {
  RegisteredBrowserEventCapability,
  RegisteredBrowserEventDispatch,
  RegisteredBrowserEventDisposition,
  RegisteredBrowserEventRegistration,
  PartiallyDispatchedBrowserEvent,
} from "../features/host.js";

const MAX_BINDING_BYTES = 1_024;
const MAX_EVENTS = 64;
const MAX_EVENT_TARGETS = 16;
const MAX_PAYLOAD_BYTES = 32 * 1_024;
const MAX_PAYLOAD_DEPTH = 32;
const MAX_PAYLOAD_ENTRIES = 2_048;
const OPERATION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const PAYLOAD_CONTRACT = /^[a-z][a-z0-9._/-]{0,127}$/u;

export interface RegisteredEventTargetResolver {
  current(): boolean;
  event(type: string, detail: JsonValue): Event;
  targets(
    target: string,
    maximumFanout: number,
  ): readonly GuardedRegisteredEventTarget[] | "fanout_exceeded";
}

export interface GuardedRegisteredEventTarget {
  current(): boolean;
  dispatch(event: Event): boolean;
}

interface AuthorityRecord {
  activeDepth: Map<string, number>;
  readonly contracts: ReadonlyMap<string, AsyncRegisteredEventContract>;
  readonly owner: object;
  readonly resolver: RegisteredEventTargetResolver;
}

function partial(
  delivered: number,
  skipped: number,
  reason: PartiallyDispatchedBrowserEvent["reason"],
): PartiallyDispatchedBrowserEvent {
  return Object.freeze({ delivered, kind: "partially_dispatched", reason, skipped });
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

function validTarget(target: string): boolean {
  return (
    target === "self" ||
    target === "parent" ||
    target === "child" ||
    target === "document" ||
    /^named_island:[a-z][a-z0-9._-]{0,63}$/u.test(target) ||
    /^browser:[a-z][a-z0-9._-]{0,63}$/u.test(target)
  );
}

function ownDataValues(input: unknown, keys: readonly string[]): readonly unknown[] | null {
  if ((typeof input !== "object" && typeof input !== "function") || input === null) return null;
  const descriptors = Object.getOwnPropertyDescriptors(input) as unknown as Record<
    PropertyKey,
    PropertyDescriptor | undefined
  >;
  const ownKeys = Reflect.ownKeys(descriptors);
  if (
    ownKeys.length !== keys.length ||
    ownKeys.some((key) => typeof key !== "string" || !keys.includes(key))
  ) {
    return null;
  }
  const values: unknown[] = [];
  for (const key of keys) {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !("value" in descriptor)) return null;
    values.push(descriptor.value);
  }
  return values;
}

function ownDenseArray(input: unknown, maximum: number): readonly unknown[] | null {
  if (!Array.isArray(input)) return null;
  const descriptors = Object.getOwnPropertyDescriptors(input) as unknown as Record<
    PropertyKey,
    PropertyDescriptor | undefined
  >;
  const lengthDescriptor = descriptors["length"];
  if (lengthDescriptor === undefined || !("value" in lengthDescriptor)) return null;
  const length: unknown = lengthDescriptor.value as unknown;
  if (!Number.isSafeInteger(length) || typeof length !== "number" || length > maximum) return null;
  if (Reflect.ownKeys(descriptors).length !== length + 1) return null;
  const values: unknown[] = [];
  for (let index = 0; index < length; index += 1) {
    const descriptor = descriptors[index];
    if (descriptor === undefined || !("value" in descriptor)) return null;
    values.push(descriptor.value);
  }
  return values;
}

function immutableRecord(
  entries: readonly (readonly [string, unknown])[],
): Readonly<Record<string, unknown>> {
  const record = Object.create(null) as Record<string, unknown>;
  for (const [key, value] of entries) {
    Object.defineProperty(record, key, { enumerable: true, value });
  }
  return Object.freeze(record);
}

interface PayloadSnapshotBudget {
  entries: number;
}

function snapshotPayload(
  input: unknown,
  budget: PayloadSnapshotBudget,
  depth = 0,
): JsonValue | null {
  if (input === null || typeof input === "boolean" || typeof input === "string") return input;
  if (typeof input === "number") return Number.isFinite(input) ? input : null;
  if (typeof input !== "object" || depth >= MAX_PAYLOAD_DEPTH) return null;
  let descriptors: Record<PropertyKey, PropertyDescriptor | undefined>;
  let prototype: object | null;
  try {
    prototype = Object.getPrototypeOf(input) as object | null;
    descriptors = Object.getOwnPropertyDescriptors(input);
  } catch {
    return null;
  }
  if (Array.isArray(input)) {
    if (prototype !== Array.prototype) return null;
    const lengthDescriptor = descriptors["length"];
    const length =
      lengthDescriptor !== undefined && "value" in lengthDescriptor
        ? (lengthDescriptor.value as unknown)
        : null;
    if (
      typeof length !== "number" ||
      !Number.isSafeInteger(length) ||
      length < 0 ||
      budget.entries + length > MAX_PAYLOAD_ENTRIES ||
      Reflect.ownKeys(descriptors).length !== length + 1
    ) {
      return null;
    }
    budget.entries += length;
    const values: JsonValue[] = [];
    for (let index = 0; index < length; index += 1) {
      const descriptor = descriptors[index];
      if (descriptor === undefined || !("value" in descriptor)) return null;
      const value = snapshotPayload(descriptor.value, budget, depth + 1);
      if (value === null && descriptor.value !== null) return null;
      values.push(value);
    }
    return Object.freeze(values);
  }
  if (prototype !== Object.prototype && prototype !== null) return null;
  const keys = Reflect.ownKeys(descriptors);
  if (
    keys.some((key) => typeof key !== "string") ||
    budget.entries + keys.length > MAX_PAYLOAD_ENTRIES
  ) {
    return null;
  }
  budget.entries += keys.length;
  const values: [string, JsonValue][] = [];
  for (const key of keys) {
    if (typeof key !== "string") return null;
    const descriptor = descriptors[key];
    if (descriptor === undefined || !("value" in descriptor)) return null;
    const value = snapshotPayload(descriptor.value, budget, depth + 1);
    if (value === null && descriptor.value !== null) return null;
    values.push([key, value]);
  }
  return immutableRecord(values) as JsonValue;
}

function snapshotDispatch(input: unknown): RegisteredBrowserEventDispatch | null {
  if ((typeof input !== "object" && typeof input !== "function") || input === null) return null;
  const prototype = Object.getPrototypeOf(input) as object | null;
  if (prototype !== Object.prototype && prototype !== null) return null;
  const values = ownDataValues(input, ["event", "payload", "schemaVersion", "target"]);
  if (values === null) return null;
  const [event, payloadInput, schemaVersion, target] = values;
  if (
    typeof event !== "string" ||
    typeof schemaVersion !== "number" ||
    !Number.isSafeInteger(schemaVersion) ||
    typeof target !== "string"
  ) {
    return null;
  }
  const payload = snapshotPayload(payloadInput, { entries: 0 });
  if (payload === null && payloadInput !== null) return null;
  return immutableRecord([
    ["event", event],
    ["payload", payload],
    ["schemaVersion", schemaVersion],
    ["target", target],
  ]) as unknown as RegisteredBrowserEventDispatch;
}

function snapshotCycle(input: unknown): AsyncRegisteredEventContract["cycle"] | null {
  const forbid = ownDataValues(input, ["kind"]);
  if (forbid?.[0] === "forbid_repeated_island") {
    return immutableRecord([
      ["kind", "forbid_repeated_island"],
    ]) as unknown as AsyncRegisteredEventContract["cycle"];
  }
  const bounded = ownDataValues(input, ["kind", "maximumHops"]);
  const maximumHops = bounded?.[1];
  if (
    bounded?.[0] !== "maximum_hops" ||
    typeof maximumHops !== "number" ||
    !Number.isSafeInteger(maximumHops) ||
    maximumHops < 1 ||
    maximumHops > 255
  ) {
    return null;
  }
  return immutableRecord([
    ["kind", "maximum_hops"],
    ["maximumHops", maximumHops],
  ]) as unknown as AsyncRegisteredEventContract["cycle"];
}

function snapshotContract(input: unknown): AsyncRegisteredEventContract | null {
  const values = ownDataValues(input, [
    "cycle",
    "maximumFanout",
    "name",
    "order",
    "payloadContract",
    "schema",
    "source",
    "targets",
    "version",
  ]);
  if (values === null) return null;
  const [
    cycleInput,
    maximumFanout,
    name,
    order,
    payloadContract,
    schema,
    source,
    targetsInput,
    version,
  ] = values;
  const cycle = snapshotCycle(cycleInput);
  const targetValues = ownDenseArray(targetsInput, MAX_EVENT_TARGETS);
  if (cycle === null || targetValues === null || targetValues.length < 1) return null;
  if (
    !targetValues.every(
      (target): target is string => typeof target === "string" && validTarget(target),
    )
  ) {
    return null;
  }
  if (
    typeof name !== "string" ||
    !OPERATION_NAME.test(name) ||
    typeof version !== "number" ||
    !Number.isSafeInteger(version) ||
    version < 1 ||
    version > 65_535 ||
    typeof payloadContract !== "string" ||
    !PAYLOAD_CONTRACT.test(payloadContract) ||
    source !== "stream" ||
    order !== "per_source_sequence" ||
    (schema !== "json" &&
      schema !== "null" &&
      schema !== "boolean" &&
      schema !== "i64" &&
      schema !== "u64" &&
      schema !== "f64" &&
      schema !== "string") ||
    new Set(targetValues).size !== targetValues.length ||
    typeof maximumFanout !== "number" ||
    !Number.isSafeInteger(maximumFanout) ||
    maximumFanout < targetValues.length ||
    maximumFanout > 256
  ) {
    return null;
  }
  return immutableRecord([
    ["cycle", cycle],
    ["maximumFanout", maximumFanout],
    ["name", name],
    ["order", "per_source_sequence"],
    ["payloadContract", payloadContract],
    ["schema", schema],
    ["source", "stream"],
    ["targets", Object.freeze([...targetValues])],
    ["version", version],
  ]) as unknown as AsyncRegisteredEventContract;
}

function snapshotRegistration(
  input: unknown,
): Readonly<{ descriptorBinding: string; events: readonly AsyncRegisteredEventContract[] }> | null {
  const values = ownDataValues(input, ["descriptorBinding", "events"]);
  const descriptorBinding = values?.[0];
  const eventValues = ownDenseArray(values?.[1], MAX_EVENTS);
  if (typeof descriptorBinding !== "string" || eventValues === null) return null;
  const bindingBytes = new TextEncoder().encode(descriptorBinding).byteLength;
  if (bindingBytes < 1 || bindingBytes > MAX_BINDING_BYTES) return null;
  const events: AsyncRegisteredEventContract[] = [];
  for (const event of eventValues) {
    const snapshot = snapshotContract(event);
    if (snapshot === null) return null;
    events.push(snapshot);
  }
  if (new Set(events.map(({ name }) => name)).size !== events.length) return null;
  return immutableRecord([
    ["descriptorBinding", descriptorBinding],
    ["events", Object.freeze(events)],
  ]) as Readonly<{
    descriptorBinding: string;
    events: readonly AsyncRegisteredEventContract[];
  }>;
}

/** Core-owned one-use-at-a-time event authority. Optional features receive only opaque capabilities. */
export class RegisteredEventAuthority {
  readonly #capabilities = new WeakMap<object, AuthorityRecord>();
  readonly #current = new WeakMap<object, object>();

  replace(
    owner: object,
    registration: RegisteredBrowserEventRegistration,
    resolver: RegisteredEventTargetResolver,
  ): RegisteredBrowserEventCapability {
    let snapshot: ReturnType<typeof snapshotRegistration>;
    try {
      snapshot = snapshotRegistration(registration);
    } catch {
      throw new Error("registered_event_authority_invalid");
    }
    if (snapshot === null) throw new Error("registered_event_authority_invalid");
    const capability = Object.freeze({}) as RegisteredBrowserEventCapability;
    const contracts = new Map(snapshot.events.map((contract) => [contract.name, contract]));
    this.#capabilities.set(capability, {
      activeDepth: new Map(),
      contracts,
      owner,
      resolver,
    });
    this.#current.set(owner, capability);
    return capability;
  }

  retire(owner: object): void {
    this.#current.delete(owner);
  }

  dispatch(
    owner: object,
    capability: RegisteredBrowserEventCapability,
    event: RegisteredBrowserEventDispatch,
  ): RegisteredBrowserEventDisposition {
    let candidate: RegisteredBrowserEventDispatch | null;
    try {
      candidate = snapshotDispatch(event);
    } catch {
      return "rejected";
    }
    if (candidate === null) return "rejected";
    const token = capability as object;
    const authority = this.#capabilities.get(token);
    if (authority === undefined) return "rejected";
    if (authority.owner !== owner) return "rejected";
    if (!authority.resolver.current()) return "retired";
    if (this.#current.get(authority.owner) !== token) return "rejected";
    const contract = authority.contracts.get(candidate.event);
    if (
      candidate.schemaVersion !== contract?.version ||
      !contract.targets.includes(candidate.target) ||
      !schemaMatches(contract.schema, candidate.payload)
    ) {
      return "rejected";
    }
    try {
      if (
        new TextEncoder().encode(canonicalize(candidate.payload)).byteLength > MAX_PAYLOAD_BYTES
      ) {
        return "rejected";
      }
    } catch {
      return "rejected";
    }
    const depth = authority.activeDepth.get(contract.name) ?? 0;
    if (
      (contract.cycle.kind === "forbid_repeated_island" && depth !== 0) ||
      (contract.cycle.kind === "maximum_hops" && depth >= contract.cycle.maximumHops)
    ) {
      return "rejected";
    }
    const targets = authority.resolver.targets(candidate.target, contract.maximumFanout);
    if (!authority.resolver.current()) return "retired";
    if (this.#current.get(authority.owner) !== token) return "rejected";
    if (targets === "fanout_exceeded") return "fanout_exceeded";
    if (targets.length === 0) return "no_target";
    if (targets.length > contract.maximumFanout) return "fanout_exceeded";
    authority.activeDepth.set(contract.name, depth + 1);
    let dispatched = 0;
    let skipped = 0;
    try {
      for (const target of targets) {
        let domEvent: Event;
        try {
          domEvent = authority.resolver.event(`suprnova:${contract.name}`, candidate.payload);
        } catch {
          return dispatched === 0
            ? "rejected"
            : partial(dispatched, targets.length - dispatched, "dispatch_failed");
        }
        if (!authority.resolver.current())
          return dispatched === 0
            ? "retired"
            : partial(dispatched, targets.length - dispatched, "source_retired");
        if (this.#current.get(authority.owner) !== token) {
          return dispatched === 0
            ? "rejected"
            : partial(dispatched, targets.length - dispatched, "capability_rotated");
        }
        if (!target.current()) {
          skipped += 1;
          continue;
        }
        if (!authority.resolver.current())
          return dispatched === 0
            ? "retired"
            : partial(dispatched, targets.length - dispatched, "source_retired");
        if (this.#current.get(authority.owner) !== token) {
          return dispatched === 0
            ? "rejected"
            : partial(dispatched, targets.length - dispatched, "capability_rotated");
        }
        target.dispatch(domEvent);
        dispatched += 1;
      }
    } catch {
      return dispatched === 0
        ? "rejected"
        : partial(dispatched, targets.length - dispatched, "dispatch_failed");
    } finally {
      if (depth === 0) authority.activeDepth.delete(contract.name);
      else authority.activeDepth.set(contract.name, depth);
    }
    if (dispatched === 0) return "no_target";
    return skipped === 0 ? "dispatched" : partial(dispatched, skipped, "target_retired");
  }
}
