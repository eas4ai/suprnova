export type SignalValue = boolean | string | number | null;

export interface SignalDeclaration {
  readonly name: string;
  readonly initial: SignalValue;
}

const SIGNAL_NAME = /^[A-Za-z][A-Za-z0-9_-]{0,127}$/u;
const INTEGER = /^-?(?:0|[1-9][0-9]{0,15})$/u;
const MAX_SIGNAL_DECLARATIONS = 32;
const MAX_SIGNAL_LITERAL_UNITS = 128;

export function parseSignalLiteral(value: string): SignalValue {
  if (value.length === 0 || value.length > MAX_SIGNAL_LITERAL_UNITS) {
    throw new Error("signal_literal_invalid");
  }
  if (value === "true") return true;
  if (value === "false") return false;
  if (value === "null") return null;
  if (INTEGER.test(value)) {
    const integer = Number(value);
    if (Number.isSafeInteger(integer)) return integer;
    throw new Error("signal_literal_invalid");
  }
  if (SIGNAL_NAME.test(value)) return value;
  throw new Error("signal_literal_invalid");
}

export function parseSignalDeclarations(value: string): readonly SignalDeclaration[] {
  const entries = value.split(",");
  if (entries.length === 0 || entries.length > MAX_SIGNAL_DECLARATIONS) {
    throw new Error("signal_declaration_limit");
  }
  const seen = new Set<string>();
  const declarations: SignalDeclaration[] = [];
  for (const entry of entries) {
    const separator = entry.indexOf(":");
    if (separator <= 0 || separator !== entry.lastIndexOf(":")) {
      throw new Error("signal_declaration_invalid");
    }
    const name = entry.slice(0, separator);
    if (!SIGNAL_NAME.test(name)) throw new Error("signal_declaration_invalid");
    if (seen.has(name)) throw new Error("signal_declaration_duplicate");
    seen.add(name);
    declarations.push(
      Object.freeze({ name, initial: parseSignalLiteral(entry.slice(separator + 1)) }),
    );
  }
  return Object.freeze(declarations);
}
