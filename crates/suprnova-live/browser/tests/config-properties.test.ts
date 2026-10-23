import fc from "fast-check";
import { describe, expect, it } from "vitest";

import {
  CONFIG_ELEMENT_ID,
  RuntimeConfigError,
  parseRuntimeConfig,
} from "../src/runtime/config.js";

const PROPERTY_SEED = 0x13579bdf;
const ERROR_CODES = new Set([
  "config_asset_identity",
  "config_credentials",
  "config_duplicate",
  "config_element_type",
  "config_endpoint",
  "config_endpoint_origin",
  "config_json",
  "config_limit",
  "config_missing",
  "config_parallel_limit",
  "config_protocol",
  "config_queue_limit",
  "config_response_limit",
  "config_shape",
  "config_timeout",
  "config_version",
]);

function documentFor(text: string): Document {
  const element = {
    textContent: text,
    getAttribute: (name: string) => (name === "type" ? "application/json" : null),
  };
  return {
    baseURI: "https://app.example.test/account",
    querySelectorAll(selector: string) {
      expect(selector).toBe(`[id="${CONFIG_ELEMENT_ID}"]`);
      return [element];
    },
  } as unknown as Document;
}

describe("runtime configuration properties", () => {
  it("classifies arbitrary nested JSON with closed errors and no prototype mutation", () => {
    fc.assert(
      fc.property(fc.json({ maxDepth: 12 }), (source) => {
        const before = (Object.prototype as { suprnovaPolluted?: unknown }).suprnovaPolluted;
        try {
          const parsed = parseRuntimeConfig(documentFor(source));
          expect(Object.isFrozen(parsed)).toBe(true);
          expect(parsed.runtimeContractVersion).toBe(1);
        } catch (error: unknown) {
          expect(error).toBeInstanceOf(RuntimeConfigError);
          if (!(error instanceof RuntimeConfigError)) throw error;
          expect(ERROR_CODES.has(error.code)).toBe(true);
          expect(error.source).toBe("document_config");
          expect(JSON.stringify(error).length).toBeLessThanOrEqual(128);
        }
        expect((Object.prototype as { suprnovaPolluted?: unknown }).suprnovaPolluted).toBe(before);
      }),
      { numRuns: 300, seed: PROPERTY_SEED },
    );
  });

  it("does not echo hostile unknown fields or honor __proto__ as configuration", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 256 }), (value) => {
        const secret = `raw-secret:${value}:end`;
        const source = `{"__proto__":{"suprnovaPolluted":true},"raw":"${JSON.stringify(secret).slice(1, -1)}"}`;
        try {
          parseRuntimeConfig(documentFor(source));
          throw new Error("hostile configuration unexpectedly accepted");
        } catch (error: unknown) {
          expect(error).toBeInstanceOf(RuntimeConfigError);
          expect(JSON.stringify(error)).not.toContain(secret);
        }
        expect(({} as { suprnovaPolluted?: unknown }).suprnovaPolluted).toBeUndefined();
      }),
      { numRuns: 200, seed: PROPERTY_SEED + 1 },
    );
  });
});
