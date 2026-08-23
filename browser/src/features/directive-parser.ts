import {
  featureDirectiveContract,
  type DirectiveCapability,
  type DirectiveFallback,
} from "../generated/directive-contract.js";
import {
  containsDynamicStructure,
  directiveName,
  MAX_ATTRIBUTE_NAME_UNITS,
  MAX_PRESENT_DIRECTIVES,
  MAX_VALUE_UNITS,
  normalizeModifiers,
  valueDiagnostic,
} from "../directives/parser.js";
import type { DirectiveDiagnosticCode } from "../directives/types.js";

export type FeatureDirectiveDiagnosticCode = DirectiveDiagnosticCode | "unsupported_modifier";

export interface ParsedFeatureDirective {
  readonly ok: true;
  readonly name: string;
  readonly value: string;
  readonly role: string | null;
  readonly modifiers: readonly string[];
  readonly capability: DirectiveCapability;
}

export interface FeatureDirectiveDiagnostic {
  readonly ok: false;
  readonly code: FeatureDirectiveDiagnosticCode;
  readonly fallback: DirectiveFallback;
}

export type FeatureDirectiveParseResult = ParsedFeatureDirective | FeatureDirectiveDiagnostic;

function diagnostic(
  code: FeatureDirectiveDiagnosticCode,
  fallback: DirectiveFallback = "inert",
): FeatureDirectiveDiagnostic {
  return { ok: false, code, fallback };
}

export function parseFeatureDirective(
  attributeName: string,
  value: string,
  presentDirectiveNames: readonly string[] = [],
): FeatureDirectiveParseResult {
  if (attributeName.length > MAX_ATTRIBUTE_NAME_UNITS || value.length > MAX_VALUE_UNITS) {
    return diagnostic("attribute_limit");
  }
  if (containsDynamicStructure(attributeName)) return diagnostic("dynamic_structure_unproved");
  if (!attributeName.startsWith("live:")) return diagnostic("not_live_directive");

  const parts = attributeName.slice(5).split(".");
  const name = parts.shift() ?? "";
  const contract = featureDirectiveContract(name);
  if (contract === undefined) return diagnostic("unknown_directive");
  const [, valueKind, allowedModifiers, allowedRoles, conflicts, fallbackCode, capability] =
    contract;
  const fallback = (["inert", "native", "retain_dom"] as const)[fallbackCode];
  let role: string | null = null;
  const firstSuffix = parts[0];
  if (firstSuffix !== undefined && allowedRoles.includes(firstSuffix)) {
    role = parts.shift() ?? null;
  }
  const modifiers = normalizeModifiers(allowedModifiers, parts);
  if (modifiers === undefined) return diagnostic("unsupported_modifier", fallback);
  if (new Set(modifiers).size !== modifiers.length) {
    return diagnostic("repeated_modifier", fallback);
  }
  if (
    presentDirectiveNames.length > MAX_PRESENT_DIRECTIVES ||
    presentDirectiveNames.some((candidate) => candidate.length > MAX_ATTRIBUTE_NAME_UNITS)
  ) {
    return diagnostic("attribute_limit", fallback);
  }
  if (presentDirectiveNames.some((candidate) => conflicts.includes(directiveName(candidate)))) {
    return diagnostic("directive_conflict", fallback);
  }
  const invalidValue = valueDiagnostic(valueKind, fallback, value);
  if (invalidValue !== null) return invalidValue;

  return { ok: true, name, value, role, modifiers, capability };
}
