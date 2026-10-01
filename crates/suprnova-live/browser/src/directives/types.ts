import type { DirectiveFallback } from "../generated/directive-contract.js";

export type DirectiveDiagnosticCode =
  | "not_live_directive"
  | "attribute_limit"
  | "unknown_directive"
  | "reserved_directive"
  | "invalid_modifier"
  | "repeated_modifier"
  | "invalid_value"
  | "unsafe_target"
  | "directive_conflict"
  | "dynamic_structure_unproved";

export interface ParsedDirective {
  readonly ok: true;
  readonly name: string;
  readonly value: string;
  readonly modifiers: readonly string[];
}

export interface DirectiveDiagnostic {
  readonly ok: false;
  readonly code: DirectiveDiagnosticCode;
  readonly fallback: DirectiveFallback;
}

export type DirectiveParseResult = ParsedDirective | DirectiveDiagnostic;
