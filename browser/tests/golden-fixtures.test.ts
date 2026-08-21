import { describe, expect, it } from "vitest";

import {
  expectedFixtureManifestSha256,
  fixtureManifestSha256,
  loadFixtureSet,
} from "../src/conformance.js";
import { CanonicalError, canonicalize, parseCanonicalJson } from "../src/canonical.js";
import { verifySnapshotFixture } from "../src/crypto.js";
import { applicationPlan } from "../src/ordering.js";
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
