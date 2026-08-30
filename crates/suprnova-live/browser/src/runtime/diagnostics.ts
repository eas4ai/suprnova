import { MAX_DIAGNOSTIC_ENTRIES, MAX_DIAGNOSTIC_SEQUENCE, boundedInteger } from "./limits.js";
import type { DiagnosticMode } from "./types.js";

export const DIAGNOSTIC_CODES = [
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
  "resource_limit",
] as const;

export const DIAGNOSTIC_SEVERITIES = ["error", "warning", "info"] as const;
export const DIAGNOSTIC_PHASES = [
  "configuration",
  "discovery",
  "directive",
  "schedule",
  "transport",
  "response",
  "morph",
  "effect",
  "navigation",
  "lifecycle",
] as const;
export const DIAGNOSTIC_DETAILS = [
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
  "disconnected",
] as const;

export type RuntimeDiagnosticCode = (typeof DIAGNOSTIC_CODES)[number];
export type RuntimeDiagnosticSeverity = (typeof DIAGNOSTIC_SEVERITIES)[number];
export type RuntimeDiagnosticPhase = (typeof DIAGNOSTIC_PHASES)[number];
export type RuntimeDiagnosticDetail = (typeof DIAGNOSTIC_DETAILS)[number];

export interface RuntimeDiagnosticInput {
  readonly code: RuntimeDiagnosticCode;
  readonly severity: RuntimeDiagnosticSeverity;
  readonly phase: RuntimeDiagnosticPhase;
  readonly detailCode: RuntimeDiagnosticDetail;
}

export interface RuntimeDiagnostic extends RuntimeDiagnosticInput {
  readonly sequence: number;
}

export interface RuntimeDiagnosticsOptions {
  readonly mode: DiagnosticMode;
  readonly maxEntries?: number;
  readonly initialSequence?: number;
  readonly emit?: (diagnostic: RuntimeDiagnostic) => void;
}

export interface RuntimeDiagnosticSink {
  record(input: RuntimeDiagnosticInput, unsafeContext?: unknown): void;
}

function contains<const Values extends readonly string[]>(
  values: Values,
  candidate: unknown,
): candidate is Values[number] {
  return typeof candidate === "string" && values.some((value) => value === candidate);
}

function validInput(input: unknown): input is RuntimeDiagnosticInput {
  const candidate = input as Partial<RuntimeDiagnosticInput> | null;
  return (
    candidate !== null &&
    typeof candidate === "object" &&
    contains(DIAGNOSTIC_CODES, candidate.code) &&
    contains(DIAGNOSTIC_SEVERITIES, candidate.severity) &&
    contains(DIAGNOSTIC_PHASES, candidate.phase) &&
    contains(DIAGNOSTIC_DETAILS, candidate.detailCode)
  );
}

export class CoreRuntimeDiagnostics implements RuntimeDiagnosticSink {
  readonly #mode: DiagnosticMode;
  #sequence = 0;

  constructor(mode: unknown) {
    if (!contains(["off", "errors", "verbose"] as const, mode)) {
      throw new RangeError("runtime_diagnostic_mode");
    }
    this.#mode = mode;
  }

  record(input: unknown, unsafeContext?: unknown): void {
    void unsafeContext;
    if (
      input === null ||
      typeof input !== "object" ||
      this.#mode === "off" ||
      (this.#mode === "errors" &&
        (input as Partial<RuntimeDiagnosticInput>).severity !== "error") ||
      this.#sequence > MAX_DIAGNOSTIC_SEQUENCE
    ) {
      return;
    }
    this.#sequence += 1;
  }
}

export class RuntimeDiagnostics implements RuntimeDiagnosticSink {
  readonly #mode: DiagnosticMode;
  readonly #maximum: number;
  readonly #emit: ((diagnostic: RuntimeDiagnostic) => void) | undefined;
  readonly #entries: RuntimeDiagnostic[] = [];
  #sequence: number;

  constructor(options: RuntimeDiagnosticsOptions) {
    const maximum = options.maxEntries ?? 256;
    const sequence = options.initialSequence ?? 0;
    if (!boundedInteger(maximum, 1, MAX_DIAGNOSTIC_ENTRIES)) {
      throw new RangeError("runtime_diagnostic_limit");
    }
    if (!boundedInteger(sequence, 0, MAX_DIAGNOSTIC_SEQUENCE)) {
      throw new RangeError("runtime_diagnostic_sequence");
    }
    if (!(["off", "errors", "verbose"] as const).some((mode) => mode === options.mode)) {
      throw new RangeError("runtime_diagnostic_mode");
    }
    this.#mode = options.mode;
    this.#maximum = maximum;
    this.#sequence = sequence;
    this.#emit = options.emit;
  }

  record(input: RuntimeDiagnosticInput, unsafeContext?: unknown): RuntimeDiagnostic | null {
    void unsafeContext;
    if (
      !validInput(input) ||
      this.#mode === "off" ||
      (this.#mode === "errors" && input.severity !== "error") ||
      this.#entries.length >= this.#maximum ||
      this.#sequence > MAX_DIAGNOSTIC_SEQUENCE
    ) {
      return null;
    }
    const diagnostic = Object.freeze({
      code: input.code,
      severity: input.severity,
      phase: input.phase,
      detailCode: input.detailCode,
      sequence: this.#sequence,
    });
    this.#entries.push(diagnostic);
    try {
      this.#emit?.(diagnostic);
    } catch {
      // A host observer cannot change runtime control flow or diagnostic contents.
    }
    this.#sequence += 1;
    return diagnostic;
  }

  entries(): readonly RuntimeDiagnostic[] {
    return Object.freeze([...this.#entries]);
  }
}
