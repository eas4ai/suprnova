import fc from "fast-check";
import { describe, expect, it } from "vitest";

import {
  CoreRuntimeDiagnostics,
  DIAGNOSTIC_CODES,
  DIAGNOSTIC_DETAILS,
  DIAGNOSTIC_PHASES,
  DIAGNOSTIC_SEVERITIES,
  RuntimeDiagnostics,
  type RuntimeDiagnosticInput,
} from "../src/runtime/diagnostics.js";

const PROPERTY_SEED = 0x27182818;

describe("diagnostic redaction properties", () => {
  it("treats arbitrary forged inputs as closed data without throwing or retaining context", () => {
    fc.assert(
      fc.property(
        fc.anything({ maxDepth: 4, maxKeys: 32 }),
        fc.string({ maxLength: 512 }),
        (input, value) => {
          const secret = `raw-secret:${value}:end`;
          const full = new RuntimeDiagnostics({ maxEntries: 8, mode: "verbose" });
          const core = new CoreRuntimeDiagnostics("verbose");
          expect(() => {
            core.record(input, { secret, input });
          }).not.toThrow();
          expect(() => {
            full.record(input as RuntimeDiagnosticInput, { secret, input });
          }).not.toThrow();
          const serialized = JSON.stringify(full.entries());
          expect(serialized.length).toBeLessThanOrEqual(1_024);
          expect(serialized).not.toContain(secret);
        },
      ),
      { numRuns: 400, seed: PROPERTY_SEED },
    );
  });

  it("emits only the finite diagnostic product under arbitrary valid sequences", () => {
    const input = fc.record({
      code: fc.constantFrom(...DIAGNOSTIC_CODES),
      severity: fc.constantFrom(...DIAGNOSTIC_SEVERITIES),
      phase: fc.constantFrom(...DIAGNOSTIC_PHASES),
      detailCode: fc.constantFrom(...DIAGNOSTIC_DETAILS),
    });
    fc.assert(
      fc.property(fc.array(input, { maxLength: 64 }), (entries) => {
        const diagnostics = new RuntimeDiagnostics({ maxEntries: 16, mode: "verbose" });
        for (const entry of entries) diagnostics.record(entry);
        const output = diagnostics.entries();
        expect(output.length).toBeLessThanOrEqual(16);
        expect(output.every((entry, index) => entry.sequence === index)).toBe(true);
        expect(JSON.stringify(output).length).toBeLessThanOrEqual(4_096);
      }),
      { numRuns: 250, seed: PROPERTY_SEED + 1 },
    );
  });
});
