import {
  directiveContract,
  isReservedDirective,
  type DirectiveFallback,
} from "../generated/directive-contract.js";
import { isSignalName } from "../signals/name.js";
import type {
  ActionLiteral,
  DirectiveDiagnostic,
  DirectiveDiagnosticCode,
  DirectiveParseResult,
} from "./types.js";

export const MAX_ATTRIBUTE_NAME_UNITS = 256;
export const MAX_VALUE_UNITS = 2_048;
export const MAX_MODIFIER_SEGMENTS = 16;
export const MAX_PRESENT_DIRECTIVES = 64;
/** The most literal arguments one action directive carries, the server's schema bound. */
export const MAX_ACTION_ARGUMENTS = 128;
const ACTION_VALUE_KIND = 4;
const IDENTIFIER = /^[A-Za-z][A-Za-z0-9_.:-]{0,127}$/;
const TARGET_ID = /^#[A-Za-z][A-Za-z0-9_-]{0,127}$/;

function diagnostic(
  code: DirectiveDiagnosticCode,
  fallback: DirectiveFallback = "inert",
): DirectiveDiagnostic {
  return { ok: false, code, fallback };
}

export function containsDynamicStructure(value: string): boolean {
  return value.includes("{{") || value.includes("{%") || value.includes("${");
}

export function normalizeModifiers(
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

function validMapping(directive: string | undefined, value: string): boolean {
  if (value.length === 0) return false;
  const entries = value.split(",");
  if (entries.length > 16) return false;
  return entries.every((entry) => {
    const separator = entry.indexOf(":");
    if (separator <= 0 || separator !== entry.lastIndexOf(":")) return false;
    const key = entry.slice(0, separator);
    const mapped = entry.slice(separator + 1);
    if (directive === "signal" ? !isSignalName(key) : !IDENTIFIER.test(key)) return false;
    if (isSignalName(mapped)) return true;
    if (!/^-?(?:0|[1-9][0-9]{0,15})$/u.test(mapped)) return false;
    return Number.isSafeInteger(Number(mapped));
  });
}

export interface ActionCall {
  readonly name: string;
  readonly arguments?: readonly ActionLiteral[];
}

const JSON_SPACE = /[ \t\n\r]*/uy;
const JSON_NUMBER = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/uy;
const HEX_UNIT = /^[0-9A-Fa-f]{4}$/u;
const KEYWORDS: readonly (readonly [string, ActionLiteral])[] = [
  ["true", true],
  ["false", false],
  ["null", null],
];
const SIMPLE_ESCAPES: ReadonlyMap<string, string> = new Map([
  ['"', '"'],
  ["'", "'"],
  ["\\", "\\"],
  ["/", "/"],
  ["b", "\b"],
  ["f", "\f"],
  ["n", "\n"],
  ["r", "\r"],
  ["t", "\t"],
]);

function skipSpace(text: string, index: number): number {
  JSON_SPACE.lastIndex = index;
  JSON_SPACE.exec(text);
  return JSON_SPACE.lastIndex;
}

function hexUnit(text: string, index: number): number | null {
  const digits = text.slice(index, index + 4);
  return HEX_UNIT.test(digits) ? Number.parseInt(digits, 16) : null;
}

function readString(text: string, start: number, quote: string): [string, number] | null {
  let result = "";
  let index = start + 1;
  while (index < text.length) {
    const character = text.charAt(index);
    if (character === quote) return [result, index + 1];
    if (character === "\\") {
      const escaped = text.charAt(index + 1);
      const simple = SIMPLE_ESCAPES.get(escaped);
      if (simple !== undefined) {
        result += simple;
        index += 2;
        continue;
      }
      if (escaped !== "u") return null;
      const unit = hexUnit(text, index + 2);
      if (unit === null || (unit >= 0xdc00 && unit < 0xe000)) return null;
      index += 6;
      if (unit >= 0xd800 && unit < 0xdc00) {
        const low = text.startsWith("\\u", index) ? hexUnit(text, index + 2) : null;
        if (low === null || low < 0xdc00 || low >= 0xe000) return null;
        result += String.fromCharCode(unit, low);
        index += 6;
      } else {
        result += String.fromCharCode(unit);
      }
      continue;
    }
    if (character.charCodeAt(0) < 0x20) return null;
    result += character;
    index += 1;
  }
  return null;
}

function readLiteral(text: string, index: number): [ActionLiteral, number] | null {
  for (const [keyword, literal] of KEYWORDS) {
    if (text.startsWith(keyword, index)) return [literal, index + keyword.length];
  }
  const first = text.charAt(index);
  if (first === "'" || first === '"') return readString(text, index, first);
  JSON_NUMBER.lastIndex = index;
  const number = JSON_NUMBER.exec(text);
  if (number === null) return null;
  const parsed = Number(number[0]);
  return Number.isFinite(parsed) ? [parsed, index + number[0].length] : null;
}

/**
 * Parses an action directive value, `name` or `name(literal, ...)`, with the
 * checker's grammar: JSON numbers, strings in single or double quotes with
 * JSON escapes plus `\'`, `true`, `false`, and `null`, separated by commas,
 * with JSON whitespace around them. Nothing is evaluated.
 */
export function parseActionCall(value: string): ActionCall | null {
  const open = value.indexOf("(");
  if (open === -1) return IDENTIFIER.test(value) ? { name: value } : null;
  const name = value.slice(0, open);
  if (!IDENTIFIER.test(name) || !value.endsWith(")")) return null;
  const body = value.slice(open + 1, -1);
  const literals: ActionLiteral[] = [];
  let index = skipSpace(body, 0);
  if (index === body.length) return { name, arguments: Object.freeze(literals) };
  for (;;) {
    const literal = readLiteral(body, index);
    if (literal === null) return null;
    literals.push(literal[0]);
    if (literals.length > MAX_ACTION_ARGUMENTS) return null;
    index = skipSpace(body, literal[1]);
    if (index === body.length) return { name, arguments: Object.freeze(literals) };
    if (body.charAt(index) !== ",") return null;
    index = skipSpace(body, index + 1);
  }
}

export function valueDiagnostic(
  valueKind: 0 | 1 | 2 | 3 | 4 | 5 | 6,
  fallback: DirectiveFallback,
  value: string,
  directive?: string,
): DirectiveDiagnostic | null {
  if (containsDynamicStructure(value)) {
    return diagnostic("dynamic_structure_unproved", fallback);
  }
  switch (valueKind) {
    case 0:
      return value.length === 0 ? null : diagnostic("invalid_value", fallback);
    case 1:
    case 3:
      return IDENTIFIER.test(value) ? null : diagnostic("invalid_value", fallback);
    case 4:
      return parseActionCall(value) === null ? diagnostic("invalid_value", fallback) : null;
    case 5:
      return safeTarget(value) ? null : diagnostic("unsafe_target", fallback);
    case 6:
      return validMapping(directive, value) ? null : diagnostic("invalid_value", fallback);
    case 2:
      return /^(?:true|false|null|-?[0-9]+|[A-Za-z][A-Za-z0-9_-]{0,127})$/u.test(value)
        ? null
        : diagnostic("invalid_value", fallback);
  }
}

export function directiveName(value: string): string {
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
  const [, valueKind, allowedModifiers, conflicts, fallbackCode] = contract;
  const fallback = (["inert", "native", "retain_dom"] as const)[fallbackCode];
  const modifiers = normalizeModifiers(allowedModifiers, parts);
  if (modifiers === undefined) return diagnostic("invalid_modifier", fallback);
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
  const invalidValue = valueDiagnostic(valueKind, fallback, value, name);
  if (invalidValue !== null) return invalidValue;
  if (valueKind === ACTION_VALUE_KIND) {
    const call = parseActionCall(value);
    if (call === null) return diagnostic("invalid_value", fallback);
    return call.arguments === undefined
      ? { ok: true, name, value: call.name, modifiers }
      : { ok: true, name, value: call.name, modifiers, arguments: call.arguments };
  }

  return { ok: true, name, value, modifiers };
}
