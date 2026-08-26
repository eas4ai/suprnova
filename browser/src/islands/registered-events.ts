import { canonicalize, type JsonValue } from "../canonical.js";
import type { AsyncPayloadSchema, AsyncRegisteredEventContract } from "../async-updates/types.js";
import type {
  RegisteredBrowserEventCapability,
  RegisteredBrowserEventDispatch,
  RegisteredBrowserEventDisposition,
  RegisteredBrowserEventRegistration,
} from "../features/host.js";

const MAX_BINDING_BYTES = 1_024;
const MAX_EVENTS = 64;
const MAX_EVENT_TARGETS = 16;
const MAX_PAYLOAD_BYTES = 32 * 1_024;
const OPERATION_NAME = /^[a-z][a-z0-9._-]{0,63}$/u;
const PAYLOAD_CONTRACT = /^[a-z][a-z0-9._/-]{0,127}$/u;

export interface RegisteredEventTargetResolver {
  current(): boolean;
  event(type: string, detail: JsonValue): Event;
  targets(target: string, maximumFanout: number): readonly EventTarget[] | "fanout_exceeded";
}

interface AuthorityRecord {
  activeDepth: Map<string, number>;
  readonly contracts: ReadonlyMap<string, AsyncRegisteredEventContract>;
  readonly owner: object;
  readonly resolver: RegisteredEventTargetResolver;
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

function validateContract(contract: AsyncRegisteredEventContract): boolean {
  const cycle = contract.cycle;
  const cycleKind: unknown = Reflect.get(cycle, "kind");
  const maximumHops: unknown = Reflect.get(cycle, "maximumHops");
  const order: unknown = Reflect.get(contract, "order");
  const source: unknown = Reflect.get(contract, "source");
  return (
    OPERATION_NAME.test(contract.name) &&
    Number.isSafeInteger(contract.version) &&
    contract.version >= 1 &&
    contract.version <= 65_535 &&
    PAYLOAD_CONTRACT.test(contract.payloadContract) &&
    source === "stream" &&
    order === "per_source_sequence" &&
    contract.targets.length >= 1 &&
    contract.targets.length <= MAX_EVENT_TARGETS &&
    new Set(contract.targets).size === contract.targets.length &&
    contract.targets.every(validTarget) &&
    Number.isSafeInteger(contract.maximumFanout) &&
    contract.maximumFanout >= contract.targets.length &&
    contract.maximumFanout <= 256 &&
    (cycleKind === "forbid_repeated_island" ||
      (cycleKind === "maximum_hops" &&
        Number.isSafeInteger(maximumHops) &&
        typeof maximumHops === "number" &&
        maximumHops >= 1 &&
        maximumHops <= 255))
  );
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
    let valid: boolean;
    try {
      valid =
        new TextEncoder().encode(registration.descriptorBinding).byteLength >= 1 &&
        new TextEncoder().encode(registration.descriptorBinding).byteLength <= MAX_BINDING_BYTES &&
        registration.events.length <= MAX_EVENTS &&
        new Set(registration.events.map(({ name }) => name)).size === registration.events.length &&
        registration.events.every(validateContract);
    } catch {
      throw new Error("registered_event_authority_invalid");
    }
    if (!valid) throw new Error("registered_event_authority_invalid");
    const capability = Object.freeze({}) as RegisteredBrowserEventCapability;
    const contracts = new Map(
      registration.events.map((contract) => [
        contract.name,
        Object.freeze({
          ...contract,
          cycle: Object.freeze({ ...contract.cycle }),
          targets: Object.freeze([...contract.targets]),
        }),
      ]),
    );
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
    const token = capability as object;
    const authority = this.#capabilities.get(token);
    if (authority === undefined) return "rejected";
    if (authority.owner !== owner) return "rejected";
    if (!authority.resolver.current()) return "retired";
    if (this.#current.get(authority.owner) !== token) return "rejected";
    const contract = authority.contracts.get(event.event);
    if (
      event.schemaVersion !== contract?.version ||
      !contract.targets.includes(event.target) ||
      !schemaMatches(contract.schema, event.payload)
    ) {
      return "rejected";
    }
    try {
      if (new TextEncoder().encode(canonicalize(event.payload)).byteLength > MAX_PAYLOAD_BYTES) {
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
    const targets = authority.resolver.targets(event.target, contract.maximumFanout);
    if (!authority.resolver.current()) return "retired";
    if (this.#current.get(authority.owner) !== token) return "rejected";
    if (targets === "fanout_exceeded") return "fanout_exceeded";
    if (targets.length === 0) return "no_target";
    if (targets.length > contract.maximumFanout) return "fanout_exceeded";
    authority.activeDepth.set(contract.name, depth + 1);
    try {
      for (const target of targets) {
        if (!authority.resolver.current()) return "retired";
        if (this.#current.get(authority.owner) !== token) return "rejected";
        target.dispatchEvent(authority.resolver.event(`suprnova:${contract.name}`, event.payload));
      }
    } catch {
      return "rejected";
    } finally {
      if (depth === 0) authority.activeDepth.delete(contract.name);
      else authority.activeDepth.set(contract.name, depth);
    }
    return "dispatched";
  }
}
