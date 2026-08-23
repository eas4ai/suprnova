import {
  directiveContract,
  isReservedDirective,
  type DirectiveFallback,
} from "../generated/directive-contract.js";
import type {
  DirectiveDiagnostic,
  DirectiveDiagnosticCode,
  DirectiveParseResult,
} from "./types.js";

const MAX_ATTRIBUTE_NAME_UNITS = 256;
const MAX_VALUE_UNITS = 2_048;
const MAX_MODIFIER_SEGMENTS = 16;
const MAX_PRESENT_DIRECTIVES = 64;
const IDENTIFIER = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/;
const TARGET_ID = /^#[A-Za-z][A-Za-z0-9_-]{0,127}$/;

function diagnostic(
  code: DirectiveDiagnosticCode,
  fallback: DirectiveFallback = "inert",
): DirectiveDiagnostic {
  return { ok: false, code, fallback };
}

function containsDynamicStructure(value: string): boolean {
  return value.includes("{{") || value.includes("{%") || value.includes("${");
}

function normalizeModifiers(
  allowedModifiers: readonly string[],
  segments: readonly string[],
): readonly string[] | undefined {
  if (segments.length > MAX_MODIFIER_SEGMENTS || segments.some((segment) => segment.length === 0)) {
    return undefined;
  }
  const normalized: string[] = [];
  for (let index = 0; index < segments.length;) {
    let matched: string | undefined;
    let consumed = 0;
    const maximum = Math.min(3, segments.length - index);
    for (let width = maximum; width >= 1; width -= 1) {
      const candidate = segments.slice(index, index + width).join(".");
      if (allowedModifiers.includes(candidate)) {
        matched = candidate;
        consumed = width;
        break;
      }
    }
    if (matched === undefined) return undefined;
    normalized.push(matched);
    index += consumed;
  }
  return normalized;
}

function safeTarget(value: string): boolean {
  if (IDENTIFIER.test(value) || TARGET_ID.test(value)) return true;
  return (
    value.startsWith("/") &&
    !value.startsWith("//") &&
    !value.includes("\\") &&
    !hasControlCharacter(value)
  );
}

function hasControlCharacter(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

function validMapping(value: string): boolean {
  if (value.length === 0) return false;
  const entries = value.split(",");
  if (entries.length > 16) return false;
  return entries.every((entry) => {
    const separator = entry.indexOf(":");
    if (separator <= 0 || separator !== entry.lastIndexOf(":")) return false;
    const key = entry.slice(0, separator);
    const mapped = entry.slice(separator + 1);
    if (!IDENTIFIER.test(key)) return false;
    if (IDENTIFIER.test(mapped)) return true;
    if (!/^-?(?:0|[1-9][0-9]{0,15})$/u.test(mapped)) return false;
    return Number.isSafeInteger(Number(mapped));
  });
}

function valueDiagnostic(
  valueKind: 0 | 1 | 2 | 3 | 4 | 5 | 6,
  fallback: DirectiveFallback,
  value: string,
): DirectiveDiagnostic | null {
  if (containsDynamicStructure(value)) {
    return diagnostic("dynamic_structure_unproved", fallback);
  }
  switch (valueKind) {
    case 0:
      return value.length === 0 ? null : diagnostic("invalid_value", fallback);
    case 1:
    case 3:
    case 4:
      return IDENTIFIER.test(value) ? null : diagnostic("invalid_value", fallback);
    case 5:
      return safeTarget(value) ? null : diagnostic("unsafe_target", fallback);
    case 6:
      return validMapping(value) ? null : diagnostic("invalid_value", fallback);
    case 2:
      return /^(?:true|false|null|-?[0-9]+|[A-Za-z][A-Za-z0-9_-]{0,127})$/u.test(value)
        ? null
        : diagnostic("invalid_value", fallback);
  }
}

function directiveName(value: string): string {
  const suffix = value.startsWith("live:") ? value.slice(5) : value;
  return suffix.split(".", 1)[0] ?? "";
}

export function parseDirective(
  attributeName: string,
  value: string,
  presentDirectiveNames: readonly string[] = [],
): DirectiveParseResult {
  if (attributeName.length > MAX_ATTRIBUTE_NAME_UNITS || value.length > MAX_VALUE_UNITS) {
    return diagnostic("attribute_limit");
  }
  if (containsDynamicStructure(attributeName)) return diagnostic("dynamic_structure_unproved");
  if (!attributeName.startsWith("live:")) return diagnostic("not_live_directive");

  const parts = attributeName.slice(5).split(".");
  const name = parts.shift() ?? "";
  if (isReservedDirective(name)) return diagnostic("reserved_directive");
  const contract = directiveContract(name);
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
  if (modifiers === undefined) {
    return diagnostic(capability === null ? "invalid_modifier" : "unsupported_modifier", fallback);
  }
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

  return { ok: true, name, value, role, modifiers };
}
