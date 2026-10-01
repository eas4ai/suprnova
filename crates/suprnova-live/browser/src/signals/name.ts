/** Canonical lowercase-first, 1–64 ASCII-byte signal-name grammar. */
export const SIGNAL_NAME_PATTERN = /^[a-z][a-z0-9._-]{0,63}$/u;

export function isSignalName(value: unknown): value is string {
  return typeof value === "string" && SIGNAL_NAME_PATTERN.test(value);
}
