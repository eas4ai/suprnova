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

export interface RuntimeCallContext {
  readonly island: IslandExtensionIdentity;
  server(name: string, input: JsonValue): Promise<JsonValue>;
  local(name: string, input: JsonValue): Promise<JsonValue>;
}

export interface RuntimeCallContextInput {
  readonly island: IslandExtensionIdentity;
  active(): boolean;
  server(name: string, input: JsonValue): Promise<JsonValue>;
  local(name: string, input: JsonValue): Promise<JsonValue>;
}

export interface RuntimeCallRegistration {
  readonly name: string;
  readonly input: PayloadSchema;
  readonly output: PayloadSchema;
  run(context: RuntimeCallContext, input: JsonValue): JsonValue | Promise<JsonValue>;
}

interface CallEntry {
  readonly registration: RuntimeCallRegistration;
  readonly lease: RegistrationLease;
}

export interface RuntimeCallRegistryOptions {
  readonly scheduler: RuntimeScheduler;
  readonly diagnostics: RuntimeDiagnosticSink;
  readonly deadlineMs?: number;
}

export class RuntimeCallRegistry {
  readonly #diagnostics: RuntimeDiagnosticSink;
  readonly #runner: BoundedExtensionRunner;
  readonly #entries = new Map<string, CallEntry>();
  #disposed = false;

  constructor(options: RuntimeCallRegistryOptions) {
    this.#diagnostics = options.diagnostics;
    this.#runner = new BoundedExtensionRunner(options.scheduler, options.deadlineMs);
  }

  register(input: RuntimeCallRegistration): VoidFunction {
    if (this.#disposed) throw new ExtensionError("extension_registry_disposed");
    requireExactRegistration(input, ["name", "input", "output", "run"]);
    validateExtensionName(input.name);
    if (typeof input.run !== "function") throw new ExtensionError("extension_registration_invalid");
    assertRegistrationCapacity(this.#entries.size);
    if (this.#entries.has(input.name)) throw new ExtensionError("extension_duplicate");
    const registration = Object.freeze({
      name: input.name,
      input: compilePayloadSchema(input.input),
      output: compilePayloadSchema(input.output),
      run: input.run.bind(undefined),
    });
    const entry = { registration, lease: createLease() };
    this.#entries.set(input.name, entry);
    let removed = false;
    return () => {
      if (removed) return;
      removed = true;
      disposeLease(entry.lease);
      this.#entries.delete(input.name);
    };
  }

  async invoke(context: RuntimeCallContextInput, name: string, input: unknown): Promise<JsonValue> {
    try {
      validateExtensionName(name);
      if (this.#disposed || !context.active()) {
        throw new ExtensionError("extension_context_invalid");
      }
      const island = normalizeIslandIdentity(context.island);
      const entry = this.#entries.get(name);
      if (entry === undefined) throw new ExtensionError("extension_missing");
      const payload = validatePayload(entry.registration.input, input);
      const result = await this.#runner.run(entry.lease, async (active) => {
        const ensure = () => {
          if (!active() || !context.active()) throw new ExtensionError("extension_canceled");
        };
        const safeContext: RuntimeCallContext = Object.freeze({
          island,
          async server(operation: string, value: JsonValue): Promise<JsonValue> {
            ensure();
            validateExtensionName(operation);
            return boundedJsonValue(await context.server(operation, boundedJsonValue(value)));
          },
          async local(operation: string, value: JsonValue): Promise<JsonValue> {
            ensure();
            validateExtensionName(operation);
            return boundedJsonValue(await context.local(operation, boundedJsonValue(value)));
          },
        });
        return entry.registration.run(safeContext, payload);
      });
      if (result.status === "completed") {
        return validatePayload(entry.registration.output, result.value);
      }
      throw new ExtensionError(`extension_${result.status}`);
    } catch (error: unknown) {
      this.#diagnostics.record({
        code: "effect_failed",
        severity: "error",
        phase: "effect",
        detailCode:
          error instanceof ExtensionError && error.code === "extension_missing"
            ? "handler_missing"
            : "operation_rejected",
      });
      if (error instanceof ExtensionError) throw error;
      throw new ExtensionError("extension_payload_invalid");
    }
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    for (const entry of this.#entries.values()) disposeLease(entry.lease);
    this.#entries.clear();
  }
}
