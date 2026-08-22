import {
  directiveContract,
  isReservedDirective,
  type DirectiveContract,
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
  contract: DirectiveContract,
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
      if (contract.modifiers.includes(candidate)) {
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
    return (
      IDENTIFIER.test(key) && (IDENTIFIER.test(mapped) || /^(?:true|false|-?[0-9]+)$/u.test(mapped))
    );
  });
}

function valueDiagnostic(contract: DirectiveContract, value: string): DirectiveDiagnostic | null {
  if (containsDynamicStructure(value)) {
    return diagnostic("dynamic_structure_unproved", contract.fallback);
  }
  switch (contract.value) {
    case "empty":
      return value.length === 0 ? null : diagnostic("invalid_value", contract.fallback);
    case "identifier":
    case "field":
    case "action":
      return IDENTIFIER.test(value) ? null : diagnostic("invalid_value", contract.fallback);
    case "target":
      return safeTarget(value) ? null : diagnostic("unsafe_target", contract.fallback);
    case "mapping":
      return validMapping(value) ? null : diagnostic("invalid_value", contract.fallback);
    case "literal":
      return /^(?:true|false|null|-?[0-9]+|[A-Za-z][A-Za-z0-9_-]{0,127})$/u.test(value)
        ? null
        : diagnostic("invalid_value", contract.fallback);
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
  const modifiers = normalizeModifiers(contract, parts);
  if (modifiers === undefined) return diagnostic("invalid_modifier", contract.fallback);
  if (new Set(modifiers).size !== modifiers.length) {
    return diagnostic("repeated_modifier", contract.fallback);
  }
  if (
    presentDirectiveNames.length > MAX_PRESENT_DIRECTIVES ||
    presentDirectiveNames.some((candidate) => candidate.length > MAX_ATTRIBUTE_NAME_UNITS)
  ) {
    return diagnostic("attribute_limit", contract.fallback);
  }
  if (
    presentDirectiveNames.some((candidate) => contract.conflicts.includes(directiveName(candidate)))
  ) {
    return diagnostic("directive_conflict", contract.fallback);
  }
  const invalidValue = valueDiagnostic(contract, value);
  if (invalidValue !== null) return invalidValue;

  return { ok: true, name, value, modifiers, contract };
}
