import { describe, expect, it } from "vitest";

import {
  FIXTURE_FILES_V4,
  FIXTURE_SETS,
  expectedFixtureManifestSha256,
  fixtureManifestSha256,
  loadFixtureSet,
  loadFixtureSets,
} from "../src/conformance.js";
import { FIXTURE_FILES_V4 as PACKAGE_FIXTURE_FILES_V4 } from "../src/index.js";
import { SUPPORTED_PROTOCOL_VERSIONS } from "../src/version.js";
import { CanonicalError, canonicalize, parseCanonicalJson } from "../src/canonical.js";
import { verifySnapshotFixture } from "../src/crypto.js";
import { applicationPlan, applicationPlanV2, type ApplicationPlanInput } from "../src/ordering.js";
import {
  ProtocolValidationError,
  validateUpdateRequest,
  validateUpdateResponse,
} from "../src/protocol.js";
import { asArray, asJsonValue, asNumber, asRecord, asString, fixtureCases } from "../src/schema.js";

const TEXT_ENCODER = new TextEncoder();

function required(fixtures: ReadonlyMap<string, unknown>, name: string): unknown {
  const value = fixtures.get(name);
  if (value === undefined) throw new TypeError(`missing_fixture:${name}`);
  return value;
}

function stringArray(value: unknown): readonly string[] {
  return asArray(value).map(asString);
}

function numberArray(value: unknown): readonly number[] {
  return asArray(value).map(asNumber);
}

function assertUniqueCaseIds(root: Readonly<Record<string, unknown>>, key: string): void {
  expect(asArray(root[key]).length).toBeGreaterThan(0);
  const seen = new Set<string>();
  for (const value of asArray(root[key])) {
    const id = asString(asRecord(value)["id"]);
    expect(id.length).toBeGreaterThan(0);
    expect(seen.has(id), `${key} contains duplicate case id ${id}`).toBe(false);
    seen.add(id);
  }
}

interface JsonMetrics {
  readonly entries: number;
  readonly maximumDepth: number;
  readonly maximumStringBytes: number;
}

function jsonMetrics(value: unknown, depth = 1): JsonMetrics {
  if (Array.isArray(value)) {
    const nested = value.map((entry) => jsonMetrics(entry, depth + 1));
    return {
      entries: value.length + nested.reduce((total, metric) => total + metric.entries, 0),
      maximumDepth: Math.max(depth, ...nested.map((metric) => metric.maximumDepth)),
      maximumStringBytes: Math.max(0, ...nested.map((metric) => metric.maximumStringBytes)),
    };
  }
  if (value !== null && typeof value === "object") {
    const entries = Object.entries(value);
    const nested = entries.map(([, entry]) => jsonMetrics(entry, depth + 1));
    return {
      entries: entries.length + nested.reduce((total, metric) => total + metric.entries, 0),
      maximumDepth: Math.max(depth, ...nested.map((metric) => metric.maximumDepth)),
      maximumStringBytes: Math.max(
        0,
        ...entries.map(([key]) => TEXT_ENCODER.encode(key).byteLength),
        ...nested.map((metric) => metric.maximumStringBytes),
      ),
    };
  }
  return {
    entries: 0,
    maximumDepth: depth,
    maximumStringBytes: typeof value === "string" ? TEXT_ENCODER.encode(value).byteLength : 0,
  };
}

function assertCodecSemantics(
  root: Readonly<Record<string, unknown>>,
  casesKey: string,
  expectedLimits: Readonly<{
    maxBytes: number;
    maxDepth: number;
    maxEntries: number;
    maxStringBytes: number;
    maxPayloadBytes?: number;
  }>,
): void {
  const limits = asRecord(root["codec_limits"]);
  expect(limits).toEqual({
    max_bytes: expectedLimits.maxBytes,
    max_depth: expectedLimits.maxDepth,
    max_entries: expectedLimits.maxEntries,
    max_string_bytes: expectedLimits.maxStringBytes,
    ...(expectedLimits.maxPayloadBytes === undefined
      ? {}
      : { max_payload_bytes: expectedLimits.maxPayloadBytes }),
  });
  expect(Object.values(expectedLimits).every((limit) => limit > 0)).toBe(true);

  for (const value of asArray(root[casesKey])) {
    const fixture = asRecord(value);
    const encoded = asString(fixture["encoded"]);
    expect(TEXT_ENCODER.encode(encoded).byteLength).toBeLessThanOrEqual(expectedLimits.maxBytes);
    const decoded: unknown = JSON.parse(encoded);
    const metrics = jsonMetrics(decoded);
    expect(metrics.entries).toBeLessThanOrEqual(expectedLimits.maxEntries);
    expect(metrics.maximumDepth).toBeLessThanOrEqual(expectedLimits.maxDepth);
    expect(metrics.maximumStringBytes).toBeLessThanOrEqual(expectedLimits.maxStringBytes);
    if (expectedLimits.maxPayloadBytes !== undefined) {
      const payload = asRecord(decoded)["payload"];
      expect(
        TEXT_ENCODER.encode(canonicalize(asJsonValue(payload))).byteLength,
      ).toBeLessThanOrEqual(expectedLimits.maxPayloadBytes);
    }
    if (asString(fixture["expected"]) === "accepted") {
      expect(canonicalize(parseCanonicalJson(encoded))).toBe(encoded);
    }
  }
}

describe("shared Live v1 fixtures", () => {
  it("loads every reviewed fixture and emits one manifest hash", async () => {
    const fixtures = await loadFixtureSet();
    expect(fixtures.size).toBe(8);
    await expect(fixtureManifestSha256()).resolves.toBe(await expectedFixtureManifestSha256());
  });

  it("keeps every accepted v1 protocol fixture byte-canonical", async () => {
    const fixtures = await loadFixtureSet();
    for (const fixture of fixtureCases(required(fixtures, "protocol-success.json"))) {
      const encoded = asString(fixture["encoded"]);
      expect(canonicalize(parseCanonicalJson(encoded))).toBe(encoded);
    }
  });

  it("canonicalizes and rejects every canonical case", async () => {
    const fixtures = await loadFixtureSet();
    for (const fixture of fixtureCases(required(fixtures, "canonical-success.json"))) {
      expect(canonicalize(parseCanonicalJson(asString(fixture["input"])))).toBe(
        asString(fixture["canonical"]),
      );
    }
    for (const fixture of fixtureCases(required(fixtures, "canonical-failure.json"))) {
      try {
        parseCanonicalJson(asString(fixture["input"]));
        throw new Error("fixture unexpectedly accepted");
      } catch (error: unknown) {
        expect(error).toBeInstanceOf(CanonicalError);
        if (!(error instanceof CanonicalError)) throw error;
        expect(error.code).toBe(asString(fixture["expected_error"]));
      }
    }
  });

  it("matches Rust array-entry failure precedence", () => {
    const limits = { maxBytes: 256, maxDepth: 3, maxEntries: 1, maxStringBytes: 1 };
    try {
      parseCanonicalJson('[0,"xx"]', limits);
      throw new Error("precedence input unexpectedly accepted");
    } catch (error: unknown) {
      expect(error).toBeInstanceOf(CanonicalError);
      if (!(error instanceof CanonicalError)) throw error;
      expect(error.code).toBe("string_too_long");
    }
  });

  it("verifies Rust-produced snapshot bytes and failure classes", async () => {
    const fixtures = await loadFixtureSet();
    for (const name of ["snapshot-success.json", "snapshot-failure.json"]) {
      const root = asRecord(required(fixtures, name));
      const rootKey = asString(root["root_key_hex"]);
      for (const fixture of fixtureCases(root)) {
        const purpose = asString(fixture["purpose"]);
        if (purpose !== "seed" && purpose !== "instance") throw new TypeError("bad_purpose");
        const result = await verifySnapshotFixture(
          asJsonValue(fixture["encoded"]),
          rootKey,
          purpose,
          Number(asString(fixture["now"])),
        );
        if (name === "snapshot-success.json") expect(result).toEqual({ ok: true });
        else {
          expect(result.ok).toBe(false);
          expect(result.error).toBe(asString(fixture["expected_error"]));
        }
      }
    }
  });

  it("accepts and rejects the same protocol fixtures", async () => {
    const fixtures = await loadFixtureSet();
    const successful = fixtureCases(required(fixtures, "protocol-success.json"));
    expect(
      successful
        .filter((fixture) => asString(fixture["kind"]) === "request")
        .map((fixture) => asString(fixture["id"])),
    ).toEqual(["seed-request", "instance-request"]);
    for (const fixture of successful) {
      const validate =
        asString(fixture["kind"]) === "request" ? validateUpdateRequest : validateUpdateResponse;
      expect(() => {
        validate(asString(fixture["encoded"]));
      }).not.toThrow();
    }
    for (const fixture of fixtureCases(required(fixtures, "protocol-failure.json"))) {
      const validate =
        asString(fixture["kind"]) === "request" ? validateUpdateRequest : validateUpdateResponse;
      try {
        validate(asString(fixture["encoded"]));
        throw new Error("fixture unexpectedly accepted");
      } catch (error: unknown) {
        expect(error).toBeInstanceOf(ProtocolValidationError);
        if (!(error instanceof ProtocolValidationError)) throw error;
        expect(error.code).toBe(asString(fixture["expected_error"]));
      }
    }
  });

  it("enumerates ordering and compatibility cases", async () => {
    const fixtures = await loadFixtureSet();
    for (const fixture of fixtureCases(required(fixtures, "response-ordering.json"))) {
      const render = asString(fixture["render"]);
      const morph = asString(fixture["morph"]);
      if (render !== "redirect" && render !== "html" && render !== "no_render") {
        throw new TypeError("bad_render");
      }
      if (
        morph !== "not_attempted" &&
        morph !== "succeeded" &&
        morph !== "failed_after_acceptance"
      ) {
        throw new TypeError("bad_morph");
      }
      expect(applicationPlan(render, morph)).toEqual(fixture["expected_steps"]);
    }
    for (const fixture of fixtureCases(required(fixtures, "compatibility.json"))) {
      const compatible =
        asNumber(fixture["protocol"]) === 1 &&
        asNumber(fixture["runtime"]) === 1 &&
        asNumber(fixture["snapshot"]) === 1;
      expect(compatible ? "compatible" : "refresh_document").toBe(asString(fixture["expected"]));
    }
  });
});

describe("shared versioned Live fixtures", () => {
  it("exports version four through the package-facing barrel", () => {
    expect(PACKAGE_FIXTURE_FILES_V4).toBe(FIXTURE_FILES_V4);
  });

  it("keeps version four independent from the Live wire protocol", () => {
    expect(FIXTURE_FILES_V4).toEqual([
      "async-envelope.json",
      "compatibility.json",
      "diagnostics.json",
      "directive-grammar.json",
      "resource-lifecycle.json",
      "runtime-features.json",
      "upload-protocol.json",
    ]);
    expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1, 2]);
  });

  it("keeps version-four case identifiers unique and hard bounds closed", async () => {
    const fixtures = await loadFixtureSet(4);
    for (const [name, collections] of [
      ["compatibility.json", ["cases"]],
      ["diagnostics.json", ["redaction_cases"]],
      ["resource-lifecycle.json", ["cases"]],
      ["upload-protocol.json", ["codec_cases", "transition_cases"]],
      ["async-envelope.json", ["envelope_cases", "continuity_cases"]],
    ] as const) {
      const root = asRecord(required(fixtures, name));
      for (const collection of collections) assertUniqueCaseIds(root, collection);
    }

    const resources = asRecord(required(fixtures, "resource-lifecycle.json"));
    const bounds = asRecord(resources["bounds"]);
    expect(bounds).toEqual({ max_items: 2, max_bytes: 8, max_active: 1 });
    for (const value of asArray(resources["cases"])) {
      let retainedItems = 0;
      let retainedBytes = 0;
      let active = 0;
      for (const operationValue of asArray(asRecord(value)["operations"])) {
        const operation = asRecord(operationValue);
        switch (asString(operation["operation"])) {
          case "enqueue":
            if (operation["expected"] === "accepted") {
              retainedItems += 1;
              retainedBytes += asNumber(operation["bytes"]);
              expect(retainedItems).toBeLessThanOrEqual(asNumber(bounds["max_items"]));
              expect(retainedBytes).toBeLessThanOrEqual(asNumber(bounds["max_bytes"]));
            }
            break;
          case "acquire":
            if (operation["expected"] === "acquired") {
              active += 1;
              expect(active).toBeLessThanOrEqual(asNumber(bounds["max_active"]));
            }
            break;
          case "release":
            expect(active).toBeGreaterThan(0);
            active -= 1;
            break;
          case "retire": {
            const expected = asRecord(operation["expected"]);
            expect(expected).toMatchObject({
              drained_items: retainedItems,
              drained_bytes: retainedBytes,
              released_permits: active,
            });
            retainedItems = 0;
            retainedBytes = 0;
            active = 0;
            break;
          }
        }
      }
    }

    const features = asRecord(required(fixtures, "runtime-features.json"));
    const registry = asRecord(features["registry"]);
    expect(registry).toMatchObject({
      maximum_features: 2,
      maximum_pending_registrations: 2,
    });
    expect(asNumber(registry["maximum_features"])).toBeGreaterThan(0);
    expect(asNumber(registry["maximum_pending_registrations"])).toBeGreaterThan(0);
    expect(asArray(features["features"]).length).toBeLessThanOrEqual(
      asNumber(registry["maximum_features"]),
    );

    const diagnostics = asRecord(required(fixtures, "diagnostics.json"));
    const retention = asRecord(diagnostics["retention"]);
    expect(retention["maximum_entries"]).toBe(256);
    expect(asNumber(retention["maximum_entries"])).toBeGreaterThan(0);
    expect(asArray(diagnostics["redaction_cases"]).length).toBeLessThanOrEqual(
      asNumber(retention["maximum_entries"]),
    );
  });

  it("parses canonical encoded cases within their exact codec bounds", async () => {
    const fixtures = await loadFixtureSet(4);
    assertCodecSemantics(asRecord(required(fixtures, "upload-protocol.json")), "codec_cases", {
      maxBytes: 16_384,
      maxDepth: 8,
      maxEntries: 64,
      maxStringBytes: 4_096,
    });
    assertCodecSemantics(asRecord(required(fixtures, "async-envelope.json")), "envelope_cases", {
      maxBytes: 65_536,
      maxDepth: 8,
      maxEntries: 1_024,
      maxStringBytes: 4_096,
      maxPayloadBytes: 32_768,
    });
  });

  it("makes idempotent upload retries deterministic from their own data", async () => {
    const fixtures = await loadFixtureSet(4);
    const upload = asRecord(required(fixtures, "upload-protocol.json"));
    const retries = asArray(upload["transition_cases"])
      .map(asRecord)
      .filter((fixture) => fixture["expected"] === "existing_outcome");
    expect(retries.length).toBeGreaterThan(0);

    for (const fixture of retries) {
      const retry = asRecord(fixture["retry"]);
      const request = asRecord(retry["request"]);
      const recorded = asRecord(retry["recorded_outcome"]);
      expect(request["operation"]).toBe(fixture["operation"]);
      expect(request["expected_revision"]).toBe(fixture["expected_revision"]);
      expect(request["chunk_index"]).toBe(fixture["chunk_index"]);
      expect(request["idempotency_key"]).toBe(fixture["idempotency_key"]);
      expect(asNumber(request["chunk_index"])).toBeGreaterThanOrEqual(0);
      expect(asString(request["idempotency_key"]).length).toBeGreaterThan(0);
      expect(recorded).toMatchObject({
        disposition: "applied",
        to: fixture["to"],
        next_revision: retry["current_revision"],
      });
      expect(retry["current_revision"]).toBe(fixture["next_revision"]);
      expect(Number(asString(fixture["expected_revision"]))).toBeLessThan(
        Number(asString(retry["current_revision"])),
      );
    }
  });

  it("keeps independent protocols and promoted directive capabilities consistent", async () => {
    const fixtures = await loadFixtureSet(4);
    const upload = asRecord(required(fixtures, "upload-protocol.json"));
    const asynchronous = asRecord(required(fixtures, "async-envelope.json"));
    expect(numberArray(upload["protocol_versions"])).toEqual([1]);
    expect(numberArray(asynchronous["protocol_versions"])).toEqual([1]);
    expect(numberArray(upload["live_protocol_versions"])).toEqual([1, 2]);
    expect(numberArray(asynchronous["live_protocol_versions"])).toEqual([1, 2]);
    expect(SUPPORTED_PROTOCOL_VERSIONS).toEqual([1, 2]);

    const features = asRecord(required(fixtures, "runtime-features.json"));
    const capabilities = new Set(
      asArray(features["features"]).map((feature) => asString(asRecord(feature)["capability"])),
    );
    const grammar = asRecord(required(fixtures, "directive-grammar.json"));
    expect(grammar).toMatchObject({ schema_version: 2, contract_version: 2 });
    const directives = asArray(grammar["directives"]).map(asRecord);
    const names = new Set<string>();
    for (const directive of directives) {
      const name = asString(directive["name"]);
      expect(names.has(name)).toBe(false);
      names.add(name);
      const roles = stringArray(directive["roles"]);
      expect(new Set(roles).size).toBe(roles.length);
      if (directive["capability"] === null) expect(roles).toEqual([]);
      else expect(capabilities.has(asString(directive["capability"]))).toBe(true);
    }
    expect(
      directives
        .filter((directive) => typeof directive["capability"] === "string")
        .map((directive) => directive["name"]),
    ).toEqual(["upload", "progress", "poll", "stream"]);

    const expected = new Map<string, Readonly<{ capability: string; roles: readonly string[] }>>([
      ["upload", { capability: "uploads@1", roles: ["cancel", "retry", "remove"] }],
      ["progress", { capability: "uploads@1", roles: [] }],
      ["poll", { capability: "async@1", roles: [] }],
      ["stream", { capability: "async@1", roles: [] }],
    ]);
    for (const [name, contract] of expected) {
      const directive = directives.find((candidate) => candidate["name"] === name);
      expect(directive).toBeDefined();
      if (directive === undefined) throw new TypeError("missing_promoted_directive");
      expect(directive["capability"]).toBe(contract.capability);
      expect(stringArray(directive["roles"])).toEqual(contract.roles);
      expect(stringArray(grammar["reserved"])).not.toContain(name);
    }
    const progress = directives.find((candidate) => candidate["name"] === "progress");
    expect(progress).toMatchObject({ value: "literal" });
    const poll = directives.find((candidate) => candidate["name"] === "poll");
    expect(poll).toMatchObject({
      modifier_conflicts: [
        ["visible", "always"],
        ["5s", "15s", "30s", "60s"],
      ],
    });
    const stream = directives.find((candidate) => candidate["name"] === "stream");
    expect(stream).toMatchObject({ modifier_conflicts: [["push-only", "hybrid"]] });
  });

  it("loads every reviewed version through one catalog and verifies each manifest", async () => {
    const fixtureSets = await loadFixtureSets();
    expect(fixtureSets.size).toBe(FIXTURE_SETS.length);
    for (const fixtureSet of FIXTURE_SETS) {
      expect(fixtureSets.get(fixtureSet.version)?.size).toBe(fixtureSet.files.length);
      await expect(fixtureManifestSha256(fixtureSet.version)).resolves.toBe(
        await expectedFixtureManifestSha256(fixtureSet.version),
      );
    }
  });

  it("shares the complete browser response-application order", async () => {
    const fixtures = await loadFixtureSet(3);
    const root = required(fixtures, "response-application.json");
    const cases = fixtureCases(root);

    expect(cases.map((fixture) => asString(fixture["id"]))).toEqual([
      "v1-terminal-redirect",
      "v2-terminal-navigated",
      "accepted-html",
      "accepted-no-render",
      "reflected-url",
      "signed-child-delivery",
      "child-delivery-and-reflection",
      "failed-morph-recovery",
      "rejected",
      "refresh-required",
      "fatal",
    ]);

    for (const fixture of cases) {
      const input = asRecord(fixture["input"]) as unknown as ApplicationPlanInput;
      expect(applicationPlanV2(input)).toEqual(fixture["expected_steps"]);
    }
  });

  it("accepts and rejects every v2 protocol fixture", async () => {
    const fixtures = await loadFixtureSet(2);
    const successful = fixtureCases(required(fixtures, "protocol-success.json"));
    expect(
      successful
        .filter((fixture) => asString(fixture["kind"]) === "request")
        .map((fixture) => asString(fixture["id"])),
    ).toEqual([
      "params-changed-request",
      "lazy-complete-request",
      "fresh-render-request",
      "seed-action-request",
      "instance-action-request",
    ]);
    for (const fixture of successful) {
      const validate =
        asString(fixture["kind"]) === "request" ? validateUpdateRequest : validateUpdateResponse;
      expect(() => {
        validate(asString(fixture["encoded"]));
      }).not.toThrow();
    }
    for (const fixture of fixtureCases(required(fixtures, "protocol-failure.json"))) {
      const validate =
        asString(fixture["kind"]) === "request" ? validateUpdateRequest : validateUpdateResponse;
      try {
        validate(asString(fixture["encoded"]));
        throw new Error("fixture unexpectedly accepted");
      } catch (error: unknown) {
        expect(error).toBeInstanceOf(ProtocolValidationError);
        if (!(error instanceof ProtocolValidationError)) throw error;
        expect(error.code).toBe(asString(fixture["expected_error"]));
      }
    }
  });
});
