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

/** One JSON-style literal an action directive passes, such as `42` in `remove(42)`. */
export type ActionLiteral = string | number | boolean | null;

export interface ParsedDirective {
  readonly ok: true;
  readonly name: string;
  /** The value; for an action directive, the action's name alone. */
  readonly value: string;
  readonly modifiers: readonly string[];
  /**
   * The literal arguments an action directive writes after its name, in
   * order. Absent when the value is the bare action name.
   */
  readonly arguments?: readonly ActionLiteral[];
}

export interface DirectiveDiagnostic {
  readonly ok: false;
  readonly code: DirectiveDiagnosticCode;
  readonly fallback: DirectiveFallback;
}

export type DirectiveParseResult = ParsedDirective | DirectiveDiagnostic;
