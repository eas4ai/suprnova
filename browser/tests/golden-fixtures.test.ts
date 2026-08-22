import { describe, expect, it } from "vitest";

import {
  FIXTURE_SETS,
  expectedFixtureManifestSha256,
  fixtureManifestSha256,
  loadFixtureSet,
  loadFixtureSets,
} from "../src/conformance.js";
import { CanonicalError, canonicalize, parseCanonicalJson } from "../src/canonical.js";
import { verifySnapshotFixture } from "../src/crypto.js";
import { applicationPlan, applicationPlanV2, type ApplicationPlanInput } from "../src/ordering.js";
import {
  ProtocolValidationError,
  validateUpdateRequest,
  validateUpdateResponse,
} from "../src/protocol.js";
import { asJsonValue, asNumber, asRecord, asString, fixtureCases } from "../src/schema.js";

function required(fixtures: ReadonlyMap<string, unknown>, name: string): unknown {
  const value = fixtures.get(name);
  if (value === undefined) throw new TypeError(`missing_fixture:${name}`);
  return value;
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
    for (const fixture of fixtureCases(required(fixtures, "protocol-success.json"))) {
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
    for (const fixture of fixtureCases(required(fixtures, "protocol-success.json"))) {
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
