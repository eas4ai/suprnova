import type { JsonValue } from "../canonical.js";
import type { RuntimeDiagnosticSink } from "../runtime/diagnostics.js";
import type { RuntimeScheduler } from "../runtime/ports.js";
import {
  assertRegistrationCapacity,
  BoundedExtensionRunner,
  createLease,
  disposeLease,
  ExtensionError,
  normalizeIslandIdentity,
  requireExactRegistration,
  validateExtensionName,
  type IslandExtensionIdentity,
  type RegistrationLease,
} from "./registry.js";
import {
  boundedJsonValue,
  compilePayloadSchema,
  validatePayload,
  type PayloadSchema,
} from "./schema.js";

const MAX_EFFECTS_PER_BATCH = 32;

export interface EffectContext {
  readonly island: IslandExtensionIdentity;
  call(name: string, input: JsonValue): Promise<JsonValue>;
}

export interface EffectRegistration {
  readonly name: string;
  readonly version: number;
  readonly schema: PayloadSchema;
  readonly phase: "after_commit";
  run(context: EffectContext, payload: JsonValue): void | Promise<void>;
}

export interface EffectContextInput {
  readonly island: IslandExtensionIdentity;
  readonly phase: "before_commit" | "after_commit";
  active(): boolean;
  invokeCall(name: string, input: JsonValue, active: () => boolean): Promise<JsonValue>;
}

export interface EffectInvocation {
  readonly name: string;
  readonly version?: number;
  readonly payload: unknown;
}

export type EffectRunStatus =
  "completed" | "missing" | "invalid" | "invalid_context" | "failed" | "timeout" | "canceled";

export interface EffectRunOutcome {
  readonly name: string;
  readonly version?: number;
  readonly status: EffectRunStatus;
}

interface EffectEntry {
  readonly registration: EffectRegistration;
  readonly lease: RegistrationLease;
}

export interface EffectRegistryOptions {
  readonly scheduler: RuntimeScheduler;
  readonly diagnostics: RuntimeDiagnosticSink;
  readonly deadlineMs?: number;
}

function outcome(name: string, status: EffectRunStatus, version?: number): EffectRunOutcome {
  return Object.freeze({ name, status, ...(version === undefined ? {} : { version }) });
}

export class EffectRegistry {
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #runner: BoundedExtensionRunner;
  readonly #byKey = new Map<string, EffectEntry>();
  readonly #byName = new Map<string, Set<EffectEntry>>();
  #disposed = false;

  constructor(options: EffectRegistryOptions) {
    this.#diagnostics = options.diagnostics;
    this.#runner = new BoundedExtensionRunner(options.scheduler, options.deadlineMs);
  }

  register(input: EffectRegistration): VoidFunction {
    if (this.#disposed) throw new ExtensionError("extension_registry_disposed");
    requireExactRegistration(input, ["name", "version", "schema", "phase", "run"]);
    validateExtensionName(input.name);
    if (!Number.isSafeInteger(input.version) || input.version < 1 || input.version > 65_535) {
      throw new ExtensionError("extension_version_invalid");
    }
    const candidate = input as unknown as Readonly<Record<string, unknown>>;
    if (Reflect.get(candidate, "phase") !== "after_commit" || typeof input.run !== "function") {
      throw new ExtensionError("extension_registration_invalid");
    }
    assertRegistrationCapacity(this.#byKey.size);
    const key = `${input.name}@${String(input.version)}`;
    if (this.#byKey.has(key)) throw new ExtensionError("extension_duplicate");
    const registration = Object.freeze({
      name: input.name,
      version: input.version,
      schema: compilePayloadSchema(input.schema),
      phase: input.phase,
      run: input.run.bind(undefined),
    });
    const entry = { registration, lease: createLease() };
    this.#byKey.set(key, entry);
    const versions = this.#byName.get(input.name) ?? new Set();
    versions.add(entry);
    this.#byName.set(input.name, versions);
    let removed = false;
    return () => {
      if (removed) return;
      removed = true;
      disposeLease(entry.lease);
      this.#byKey.delete(key);
      versions.delete(entry);
      if (versions.size === 0) this.#byName.delete(input.name);
    };
  }

  async runAll(
    context: EffectContextInput,
    invocations: readonly EffectInvocation[],
  ): Promise<readonly EffectRunOutcome[]> {
    if (invocations.length > MAX_EFFECTS_PER_BATCH) {
      this.#failure("resource_exhausted");
      return Object.freeze([]);
    }
    let island: IslandExtensionIdentity;
    try {
      island = normalizeIslandIdentity(context.island);
    } catch {
      return Object.freeze(invocations.map(({ name }) => outcome(name, "invalid_context")));
    }
    if (this.#disposed || context.phase !== "after_commit" || !context.active()) {
      return Object.freeze(invocations.map(({ name }) => outcome(name, "invalid_context")));
    }
    const results: EffectRunOutcome[] = [];
    for (const invocation of invocations) {
      results.push(await this.#runOne(context, island, invocation));
    }
    return Object.freeze(results);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const entry of this.#byKey.values()) disposeLease(entry.lease);
    this.#byKey.clear();
    this.#byName.clear();
  }

  async #runOne(
    source: EffectContextInput,
    island: IslandExtensionIdentity,
    invocation: EffectInvocation,
  ): Promise<EffectRunOutcome> {
    const keys = Object.keys(invocation);
    if (
      !keys.includes("name") ||
      !keys.includes("payload") ||
      keys.some((key) => !["name", "payload", "version"].includes(key)) ||
      (invocation.version !== undefined &&
        (!Number.isSafeInteger(invocation.version) ||
          invocation.version < 1 ||
          invocation.version > 65_535))
    ) {
      this.#failure("invalid_shape");
      return outcome(invocation.name, "invalid", invocation.version);
    }
    let entry: EffectEntry | undefined;
    try {
      validateExtensionName(invocation.name);
      if (invocation.version !== undefined) {
        entry = this.#byKey.get(`${invocation.name}@${String(invocation.version)}`);
      } else {
        const candidates = [...(this.#byName.get(invocation.name) ?? [])];
        if (candidates.length === 1) entry = candidates[0];
      }
    } catch {
      // Unknown or malformed names share one closed missing-handler outcome.
    }
    if (entry === undefined) {
      this.#failure("handler_missing");
      return outcome(invocation.name, "missing", invocation.version);
    }
    const { registration, lease } = entry;
    let payload: JsonValue;
    try {
      payload = validatePayload(registration.schema, invocation.payload);
    } catch {
      this.#failure("invalid_shape");
      return outcome(invocation.name, "invalid", registration.version);
    }
    const result = await this.#runner.run(lease, async (active) => {
      const context: EffectContext = Object.freeze({
        island,
        async call(name: string, input: JsonValue): Promise<JsonValue> {
          if (!active() || !source.active()) throw new ExtensionError("extension_canceled");
          validateExtensionName(name);
          return boundedJsonValue(await source.invokeCall(name, boundedJsonValue(input), active));
        },
      });
      await registration.run(context, payload);
    });
    if (result.status !== "completed") this.#failure("operation_rejected");
    return outcome(invocation.name, result.status, registration.version);
  }

  #failure(
    detailCode: "handler_missing" | "invalid_shape" | "operation_rejected" | "resource_exhausted",
  ): void {
    this.#diagnostics.record({
      code: "effect_failed",
      severity: "error",
      phase: "effect",
      detailCode,
    });
  }
}
