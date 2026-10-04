import fc from "fast-check";
import { describe, expect, it } from "vitest";

import {
  CONFIG_ELEMENT_ID,
  RuntimeConfigError,
  parseRuntimeConfig,
} from "../src/runtime/config.js";
import { SERVER_DEFAULT_LIMITS } from "../src/limits.js";
import { resolveRuntimePorts, type RuntimePorts } from "../src/runtime/ports.js";
import type { BootstrapOptions } from "../src/runtime/types.js";

const VALID_CONFIG = {
  asset_identity: "runtime-test-v1",
  credentials: "same-origin",
  endpoint: "/_suprnova/live",
  max_html_bytes: 1_048_576,
  max_json_depth: 32,
  max_json_entries: 1_000_000,
  max_parallel_per_island: 1,
  max_queued_per_island: 16,
  max_request_bytes: 16_777_216,
  max_request_items: 65_536,
  max_response_bytes: 1_048_576,
  max_response_items: 65_536,
  morph_deadline_ms: 0,
  morph_max_attributes: 10_000_000,
  morph_max_attributes_per_element: 4_096,
  morph_max_depth: 512,
  morph_max_keys: 1_000_000,
  morph_max_nodes: 1_000_000,
  protocol: { maximum: 2, minimum: 1 },
  request_timeout_ms: 15_000,
  runtime_contract_version: 1,
};

interface FakeConfigElement {
  readonly textContent: string;
  getAttribute(name: string): string | null;
}

function configElement(text: string, type = "application/json"): FakeConfigElement {
  return {
    textContent: text,
    getAttribute(name: string) {
      return name === "type" ? type : null;
    },
  };
}

function configDocument(
  texts: readonly string[],
  options: { readonly baseURI?: string; readonly type?: string } = {},
): Document {
  const elements = texts.map((text) => configElement(text, options.type));
  return {
    baseURI: options.baseURI ?? "https://app.example.test/account",
    querySelectorAll(selector: string) {
      expect(selector).toBe(`[id="${CONFIG_ELEMENT_ID}"]`);
      return elements;
    },
  } as unknown as Document;
}

function encoded(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({ ...VALID_CONFIG, ...overrides });
}

function expectConfigFailure(
  texts: readonly string[],
  code: string,
  options: BootstrapOptions = {},
  documentOptions: { readonly baseURI?: string; readonly type?: string } = {},
): void {
  try {
    parseRuntimeConfig(configDocument(texts, documentOptions), options);
    throw new Error("config unexpectedly accepted");
  } catch (error: unknown) {
    expect(error).toBeInstanceOf(RuntimeConfigError);
    if (!(error instanceof RuntimeConfigError)) throw error;
    expect(error.code).toBe(code);
    expect(error.source).toBe("document_config");
  }
}

describe("bounded runtime configuration", () => {
  it("loads one exact same-origin application/json contract", () => {
    const config = parseRuntimeConfig(configDocument([encoded()]));

    expect(config).toMatchObject({
      runtimeContractVersion: 1,
      protocol: { minimum: 1, maximum: 2 },
      credentials: "same-origin",
      requestTimeoutMs: 15_000,
      limits: {
        ...SERVER_DEFAULT_LIMITS,
        maxHtmlBytes: 1_048_576,
        maxResponseBytes: 1_048_576,
      },
      maxQueuedPerIsland: 16,
      maxParallelPerIsland: 1,
      assetIdentity: "runtime-test-v1",
    });
    expect(config.endpoint.href).toBe("https://app.example.test/_suprnova/live");
  });

  it("rejects missing, duplicate, wrong-type, unknown, and DOM-privilege config", () => {
    expectConfigFailure([], "config_missing");
    expectConfigFailure([encoded(), encoded()], "config_duplicate");
    expectConfigFailure([encoded()], "config_element_type", {}, { type: "text/plain" });
    expectConfigFailure([encoded({ extra: true })], "config_shape");
    expectConfigFailure([encoded({ diagnostics: "verbose" })], "config_shape");
    expectConfigFailure(
      [encoded({ allowed_endpoint_origins: ["https://api.example.test"] })],
      "config_shape",
    );
  });

  it("enforces depth, versions, and every numeric bound, but no size of its own", () => {
    // The element is the server's own output; it has no byte or entry cap,
    // only the parser's depth guard.
    expect(() =>
      parseRuntimeConfig(configDocument([`${" ".repeat(64 * 1024)}${encoded()}`])),
    ).not.toThrow();
    expectConfigFailure(
      [JSON.stringify({ nested: JSON.parse(`${"[".repeat(33)}${"]".repeat(33)}`) as unknown })],
      "config_limit",
    );
    expectConfigFailure([encoded({ max_html_bytes: 0 })], "config_limit");
    expectConfigFailure([encoded({ morph_max_nodes: -1 })], "config_limit");
    expectConfigFailure([encoded({ morph_deadline_ms: -1 })], "config_limit");
    expectConfigFailure([encoded({ max_json_depth: 65 })], "config_limit");
    expectConfigFailure([encoded({ max_html_bytes: 2_097_152 })], "config_limit");
    expectConfigFailure(
      [encoded({ morph_max_attributes: 10, morph_max_attributes_per_element: 11 })],
      "config_limit",
    );
    expectConfigFailure([encoded({ runtime_contract_version: 2 })], "config_version");
    expectConfigFailure([encoded({ protocol: { minimum: 2, maximum: 1 } })], "config_protocol");
    expectConfigFailure([encoded({ credentials: "omit" })], "config_credentials");
    expectConfigFailure([encoded({ request_timeout_ms: 0 })], "config_timeout");
    expectConfigFailure([encoded({ max_response_bytes: 0 })], "config_response_limit");
    expectConfigFailure([encoded({ max_queued_per_island: 0 })], "config_queue_limit");
    expectConfigFailure([encoded({ max_parallel_per_island: 9 })], "config_parallel_limit");
  });

  it("rejects unsafe endpoints and requires trusted approval for cross-origin origins", () => {
    for (const endpoint of [
      "javascript:alert(1)",
      "//evil.example/live",
      "/unsafe\\path",
      "/unsafe\npath",
      "https://user:secret@api.example.test/live",
    ]) {
      expectConfigFailure([encoded({ endpoint })], "config_endpoint");
    }
    expectConfigFailure(
      [encoded({ endpoint: "https://api.example.test/live", credentials: "include" })],
      "config_endpoint_origin",
    );

    const config = parseRuntimeConfig(
      configDocument([
        encoded({ endpoint: "https://api.example.test/live", credentials: "include" }),
      ]),
      { allowedEndpointOrigins: ["https://api.example.test"] },
    );
    expect(config.endpoint.origin).toBe("https://api.example.test");
  });

  it("accepts the server's 16 MiB and 1 GiB response limits", () => {
    for (const bytes of [16_777_216, 1_073_741_824]) {
      const config = parseRuntimeConfig(
        configDocument([
          encoded({ max_html_bytes: bytes, max_request_bytes: bytes, max_response_bytes: bytes }),
        ]),
      );
      expect(config.limits.maxResponseBytes).toBe(bytes);
    }
  });

  it("rejects timeouts setTimeout cannot hold, and accepts long ones", () => {
    expect(
      parseRuntimeConfig(configDocument([encoded({ request_timeout_ms: 600_000 })]))
        .requestTimeoutMs,
    ).toBe(600_000);
    fc.assert(
      fc.property(
        fc.oneof(
          fc.integer({ max: 0 }),
          fc.integer({ min: 2_147_483_648, max: Number.MAX_SAFE_INTEGER }),
        ),
        (requestTimeoutMs) => {
          expectConfigFailure(
            [encoded({ request_timeout_ms: requestTimeoutMs })],
            "config_timeout",
          );
        },
      ),
      { numRuns: 100 },
    );
  });

  it("resolves every deterministic host port without document-provided authority", () => {
    const defaults = {
      clock: {},
      randomness: {},
      transport: {},
      navigation: {},
      observers: {},
      scheduler: {},
      features: {},
    } as RuntimePorts;
    const overrides = {
      clock: { name: "clock" },
      randomness: { name: "randomness" },
      transport: { name: "transport" },
      navigation: { name: "navigation" },
      observers: { name: "observers" },
      scheduler: { name: "scheduler" },
      features: { name: "features" },
    } as unknown as RuntimePorts;

    expect(resolveRuntimePorts(defaults, overrides)).toEqual(overrides);
  });

  it("keeps the legacy platform-feature override unchanged", () => {
    const platformFeatures = {
      prefersReducedMotion: () => false,
      supportsSpeculationRules: () => true,
      supportsViewTransitions: () => true,
    };
    const legacy = { features: platformFeatures } satisfies BootstrapOptions;

    expect(legacy.features).toBe(platformFeatures);
  });
});
