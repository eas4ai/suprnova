import type { LiveLimitBreach } from "../limits.js";
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
  readonly console?: DiagnosticConsole | null;
}

export interface RuntimeDiagnosticSink {
  record(input: RuntimeDiagnosticInput, unsafeContext?: unknown): void;
  /// Reports a configured limit that tripped. A closed diagnostic code says
  /// only that a limit was hit; the developer also needs which one, by how
  /// much, and the setting to raise, so this path prints the breach.
  limit?(breach: LiveLimitBreach): void;
}

/// The one console method a limit report uses, injectable for tests.
export interface DiagnosticConsole {
  error(message: string): void;
}

function defaultConsole(): DiagnosticConsole | null {
  const candidate: unknown = Reflect.get(globalThis, "console");
  return candidate !== null &&
    typeof candidate === "object" &&
    typeof Reflect.get(candidate, "error") === "function"
    ? (candidate as DiagnosticConsole)
    : null;
}

/// Writes a breach to the console unless diagnostics are off. The breach holds
/// only sizes and fixed names, never page content, so it is safe to print.
function printBreach(
  mode: DiagnosticMode,
  console: DiagnosticConsole | null,
  breach: LiveLimitBreach,
): void {
  if (mode === "off" || console === null) return;
  try {
    console.error(breach.message);
  } catch {
    // A replaced console cannot change runtime control flow.
  }
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
  readonly #console: DiagnosticConsole | null;
  #sequence = 0;

  constructor(mode: unknown, console: DiagnosticConsole | null = defaultConsole()) {
    if (!contains(["off", "errors", "verbose"] as const, mode)) {
      throw new RangeError("runtime_diagnostic_mode");
    }
    this.#mode = mode;
    this.#console = console;
  }

  limit(breach: LiveLimitBreach): void {
    printBreach(this.#mode, this.#console, breach);
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
  readonly #console: DiagnosticConsole | null;
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
    this.#console = options.console === undefined ? defaultConsole() : options.console;
  }

  limit(breach: LiveLimitBreach): void {
    printBreach(this.#mode, this.#console, breach);
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
