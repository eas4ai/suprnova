import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
import type { DirectiveDiagnosticCode } from "../src/directives/types.js";

const PROPERTY_SEED = 0x02468ace;
const DIAGNOSTIC_CODES = new Set<DirectiveDiagnosticCode>([
  "not_live_directive",
  "attribute_limit",
  "unknown_directive",
  "reserved_directive",
  "invalid_modifier",
  "unsupported_modifier",
  "repeated_modifier",
  "invalid_value",
  "unsafe_target",
  "directive_conflict",
  "dynamic_structure_unproved",
]);
const HOSTILE = [
  "__proto__",
  "constructor.prototype.polluted",
  "{{dynamic}}",
  "{% executable %}",
  "${interpolation}",
  "//evil.example/path",
  "javascript:alert(1)",
  "\0\r\n<script>secret</script>",
] as const;

const hostileString = fc.oneof(fc.string({ maxLength: 2_200 }), fc.constantFrom(...HOSTILE));

describe("directive parser properties", () => {
  it("is total and bounded for hostile names, values, modifiers, and sibling sets", () => {
    fc.assert(
      fc.property(
        hostileString,
        hostileString,
        fc.array(hostileString, { maxLength: 70 }),
        (attributeName, value, present) => {
          const before = (Object.prototype as { polluted?: unknown }).polluted;
          const result = parseDirective(attributeName, value, present);
          expect(JSON.stringify(result).length).toBeLessThanOrEqual(4_096);
          if (result.ok) {
            expect(result.name.length).toBeLessThanOrEqual(128);
            expect(result.value.length).toBeLessThanOrEqual(2_048);
            expect(result.modifiers.length).toBeLessThanOrEqual(16);
          } else {
            expect(DIAGNOSTIC_CODES.has(result.code)).toBe(true);
            expect(["inert", "native", "retain_dom"]).toContain(result.fallback);
          }
          expect((Object.prototype as { polluted?: unknown }).polluted).toBe(before);
        },
      ),
      { numRuns: 500, seed: PROPERTY_SEED },
    );
  });

  it("returns closed diagnostics without echoing rejected directive text", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 256 }), (suffix) => {
        const secret = `raw-secret-${suffix}-end`;
        const result = parseDirective(`live:unknown-${secret}`, secret, [secret]);
        expect(result.ok).toBe(false);
        expect(JSON.stringify(result)).not.toContain(secret);
      }),
      { numRuns: 250, seed: PROPERTY_SEED + 1 },
    );
  });
});
