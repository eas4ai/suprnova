import { CanonicalError, DEFAULT_CANONICAL_LIMITS, parseCanonicalJson } from "../canonical.js";
import { validLiveLimits, type LiveLimits } from "../limits.js";
import { RUNTIME_CONFIG_LIMITS, boundedInteger } from "./limits.js";
import type { BootstrapOptions, RuntimeConfig } from "./types.js";

export const CONFIG_ELEMENT_ID = "suprnova-live-config";

export const NAVIGATION_RUNTIME_LIMITS = Object.freeze({
  maxConcurrentPrefetches: 2,
  maxPrefetchTargets: 256,
});

// Every page limit the server configures, by its configuration-element key.
// The element is the only place the browser learns them (spec 09).
const LIMIT_KEYS = Object.freeze({
  async_max_queued_events: "asyncMaxQueuedEvents",
  async_max_replay_events: "asyncMaxReplayEvents",
  max_html_bytes: "maxHtmlBytes",
  max_json_depth: "maxJsonDepth",
  max_json_entries: "maxJsonEntries",
  max_redirect_bytes: "maxRedirectBytes",
  max_request_bytes: "maxRequestBytes",
  max_request_items: "maxRequestItems",
  max_response_bytes: "maxResponseBytes",
  max_response_items: "maxResponseItems",
  morph_deadline_ms: "morphDeadlineMs",
  morph_max_attributes: "morphMaxAttributes",
  morph_max_attributes_per_element: "morphMaxAttributesPerElement",
  morph_max_depth: "morphMaxDepth",
  morph_max_keys: "morphMaxKeys",
  morph_max_nodes: "morphMaxNodes",
  upload_chunk_bytes: "uploadChunkBytes",
  upload_max_active: "uploadMaxActive",
  upload_max_file_bytes: "uploadMaxFileBytes",
  upload_max_pending_bytes: "uploadMaxPendingBytes",
  upload_max_pending_files: "uploadMaxPendingFiles",
} as const satisfies Readonly<Record<string, keyof LiveLimits>>);

const CONFIG_KEYS = [
  "asset_identity",
  "credentials",
  "endpoint",
  "max_parallel_per_island",
  "max_queued_per_island",
  "protocol",
  "request_timeout_ms",
  "runtime_contract_version",
  ...Object.keys(LIMIT_KEYS),
].sort();
const PROTOCOL_KEYS = ["maximum", "minimum"] as const;

export type RuntimeConfigErrorCode =
  | "config_missing"
  | "config_duplicate"
  | "config_element_type"
  | "config_limit"
  | "config_json"
  | "config_shape"
  | "config_version"
  | "config_protocol"
  | "config_endpoint"
  | "config_endpoint_origin"
  | "config_credentials"
  | "config_timeout"
  | "config_response_limit"
  | "config_queue_limit"
  | "config_parallel_limit"
  | "config_asset_identity";

export type RuntimeConfigErrorSource = "document_config" | "bootstrap_options";

export class RuntimeConfigError extends Error {
  readonly code: RuntimeConfigErrorCode;
  readonly source: RuntimeConfigErrorSource;

  constructor(code: RuntimeConfigErrorCode, source: RuntimeConfigErrorSource = "document_config") {
    super(`runtime_config_error:${code}`);
    this.name = "RuntimeConfigError";
    this.code = code;
    this.source = source;
  }
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

function configFailure(error: unknown): RuntimeConfigError {
  if (error instanceof RuntimeConfigError) return error;
  if (error instanceof CanonicalError) {
    return new RuntimeConfigError(
      ["input_too_large", "input_too_deep", "too_many_entries", "string_too_long"].includes(
        error.code,
      )
        ? "config_limit"
        : "config_json",
    );
  }
  return new RuntimeConfigError("config_json");
}

function hasUnsafeUrlText(value: string): boolean {
  if (value.startsWith("//") || value.includes("\\")) return true;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code <= 31 || code === 127) return true;
  }
  return false;
}

function approvedOrigins(options: BootstrapOptions): ReadonlySet<string> {
  const values = options.allowedEndpointOrigins ?? [];
  if (values.length > RUNTIME_CONFIG_LIMITS.maxAllowedOrigins) {
    throw new RuntimeConfigError("config_endpoint_origin", "bootstrap_options");
  }
  const origins = new Set<string>();
  for (const value of values) {
    try {
      const url = new URL(value);
      const normalized = value.endsWith("/") ? value.slice(0, -1) : value;
      if (
        !["http:", "https:"].includes(url.protocol) ||
        url.username.length > 0 ||
        url.password.length > 0 ||
        url.origin !== normalized ||
        url.pathname !== "/" ||
        url.search.length > 0 ||
        url.hash.length > 0
      ) {
        throw new TypeError("invalid_origin");
      }
      origins.add(url.origin);
    } catch {
      throw new RuntimeConfigError("config_endpoint_origin", "bootstrap_options");
    }
  }
  return origins;
}

function endpoint(value: unknown, document: Document, options: BootstrapOptions): URL {
  if (typeof value !== "string") throw new RuntimeConfigError("config_endpoint");
  if (hasUnsafeUrlText(value)) throw new RuntimeConfigError("config_endpoint");
  let base: URL;
  let parsed: URL;
  try {
    base = new URL(document.baseURI);
    parsed = new URL(value, base);
  } catch {
    throw new RuntimeConfigError("config_endpoint");
  }
  if (
    !["http:", "https:"].includes(base.protocol) ||
    !["http:", "https:"].includes(parsed.protocol) ||
    parsed.username.length > 0 ||
    parsed.password.length > 0 ||
    parsed.hash.length > 0
  ) {
    throw new RuntimeConfigError("config_endpoint");
  }
  if (parsed.origin !== base.origin && !approvedOrigins(options).has(parsed.origin)) {
    throw new RuntimeConfigError("config_endpoint_origin");
  }
  return parsed;
}

function parseProtocol(value: unknown): Readonly<{ minimum: 1 | 2; maximum: 1 | 2 }> {
  const protocol = record(value);
  if (protocol === null || !exactKeys(protocol, PROTOCOL_KEYS)) {
    throw new RuntimeConfigError("config_protocol");
  }
  const minimum = protocol["minimum"];
  const maximum = protocol["maximum"];
  if (!protocolVersion(minimum) || !protocolVersion(maximum) || minimum > maximum) {
    throw new RuntimeConfigError("config_protocol");
  }
  return Object.freeze({ minimum, maximum });
}

/// Reads the server's page limits. The server validated them when it booted;
/// a value that is not a positive integer, or a set whose nested pairs do not
/// nest, means the element was not written by a matching server.
function parseLimits(config: Readonly<Record<string, unknown>>): LiveLimits {
  const limits: Record<string, number> = {};
  for (const [key, name] of Object.entries(LIMIT_KEYS)) {
    const value = config[key];
    if (typeof value !== "number") throw new RuntimeConfigError("config_limit");
    limits[name] = value;
  }
  const parsed = Object.freeze(limits) as unknown as LiveLimits;
  if (!validLiveLimits(parsed)) throw new RuntimeConfigError("config_limit");
  return parsed;
}

function protocolVersion(value: unknown): value is 1 | 2 {
  return value === 1 || value === 2;
}

export function parseRuntimeConfig(
  document: Document,
  options: BootstrapOptions = {},
): RuntimeConfig {
  const elements = Array.from(document.querySelectorAll(`[id="${CONFIG_ELEMENT_ID}"]`));
  if (elements.length === 0) throw new RuntimeConfigError("config_missing");
  if (elements.length !== 1) throw new RuntimeConfigError("config_duplicate");
  const element = elements[0];
  if (element?.getAttribute("type") !== "application/json") {
    throw new RuntimeConfigError("config_element_type");
  }
  const text = element.textContent;
  let parsed: unknown;
  try {
    parsed = parseCanonicalJson(text, DEFAULT_CANONICAL_LIMITS);
  } catch (error: unknown) {
    throw configFailure(error);
  }
  const config = record(parsed);
  if (config === null || !exactKeys(config, CONFIG_KEYS)) {
    throw new RuntimeConfigError("config_shape");
  }
  if (config["runtime_contract_version"] !== 1) {
    throw new RuntimeConfigError("config_version");
  }
  const credentials = config["credentials"];
  if (credentials !== "same-origin" && credentials !== "include") {
    throw new RuntimeConfigError("config_credentials");
  }
  const requestTimeoutMs = config["request_timeout_ms"];
  if (
    !boundedInteger(
      requestTimeoutMs,
      RUNTIME_CONFIG_LIMITS.minRequestTimeoutMs,
      RUNTIME_CONFIG_LIMITS.maxRequestTimeoutMs,
    )
  ) {
    throw new RuntimeConfigError("config_timeout");
  }
  if (!boundedInteger(config["max_response_bytes"], 1, Number.MAX_SAFE_INTEGER)) {
    throw new RuntimeConfigError("config_response_limit");
  }
  const limits = parseLimits(config);
  const maxQueuedPerIsland = config["max_queued_per_island"];
  if (!boundedInteger(maxQueuedPerIsland, 1, RUNTIME_CONFIG_LIMITS.maxQueuedPerIsland)) {
    throw new RuntimeConfigError("config_queue_limit");
  }
  const maxParallelPerIsland = config["max_parallel_per_island"];
  if (
    !boundedInteger(maxParallelPerIsland, 1, RUNTIME_CONFIG_LIMITS.maxParallelPerIsland) ||
    maxParallelPerIsland > maxQueuedPerIsland
  ) {
    throw new RuntimeConfigError("config_parallel_limit");
  }
  const assetIdentity = config["asset_identity"];
  if (
    typeof assetIdentity !== "string" ||
    assetIdentity.length > RUNTIME_CONFIG_LIMITS.maxAssetIdentityUnits ||
    !/^[A-Za-z0-9][A-Za-z0-9._:-]*$/u.test(assetIdentity)
  ) {
    throw new RuntimeConfigError("config_asset_identity");
  }

  return Object.freeze({
    runtimeContractVersion: 1,
    protocol: parseProtocol(config["protocol"]),
    endpoint: endpoint(config["endpoint"], document, options),
    credentials,
    requestTimeoutMs,
    limits,
    maxQueuedPerIsland,
    maxParallelPerIsland,
    assetIdentity,
  });
}
