import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
import { parseFeatureDirective } from "../src/features/directive-parser.js";
import { DIRECTIVE_CONTRACTS } from "../src/generated/directive-contract.js";

describe("iteration 004 directive parser", () => {
  it("consumes the generated capability contracts for all promoted directives", () => {
    const expected = [
      ["upload", "uploads@1", ["cancel", "retry", "remove"]],
      ["progress", "uploads@1", []],
      ["poll", "async@1", []],
      ["stream", "async@1", []],
    ] as const;

    for (const [name, capability, roles] of expected) {
      const contract = DIRECTIVE_CONTRACTS.find((candidate) => candidate.name === name);
      expect(contract).toMatchObject({ name, capability, roles });
    }
  });

  it("keeps capability directives reserved and the legacy core result shape unchanged", () => {
    expect(parseDirective("live:upload", "avatar")).toEqual({
      ok: false,
      code: "reserved_directive",
      fallback: "inert",
    });
    expect(parseDirective("live:poll.visible.30s", "refresh")).toEqual({
      ok: false,
      code: "reserved_directive",
      fallback: "inert",
    });
    expect(parseFeatureDirective("live:click", "save")).toEqual({
      ok: false,
      code: "unknown_directive",
      fallback: "inert",
    });
    const parsed = parseDirective("live:click.prevent", "save");
    expect(parsed).toEqual({
      ok: true,
      name: "click",
      value: "save",
      modifiers: ["prevent"],
    });
    expect("role" in parsed).toBe(false);
    expect("capability" in parsed).toBe(false);
  });

  it("parses every reviewed role and asynchronous modifier through the feature parser", () => {
    for (const role of ["cancel", "retry", "remove"] as const) {
      expect(parseFeatureDirective(`live:upload.${role}`, "avatar")).toEqual({
        ok: true,
        name: "upload",
        value: "avatar",
        role,
        modifiers: [],
        capability: "uploads@1",
      });
    }
    expect(parseFeatureDirective("live:progress", "avatar")).toMatchObject({
      ok: true,
      role: null,
      capability: "uploads@1",
    });
    expect(parseFeatureDirective("live:poll.visible.30s", "refresh")).toEqual({
      ok: true,
      name: "poll",
      value: "refresh",
      role: null,
      modifiers: ["visible", "30s"],
      capability: "async@1",
    });
    expect(parseFeatureDirective("live:stream.push-only", "orders")).toMatchObject({
      ok: true,
      role: null,
      modifiers: ["push-only"],
      capability: "async@1",
    });
  });

  it("rejects illegal roles and unsupported modifiers with the generated closed fallback", () => {
    expect(parseFeatureDirective("live:upload.stream", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "native",
    });
    expect(parseFeatureDirective("live:upload.cancel.retry", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "native",
    });
    expect(parseFeatureDirective("live:progress.cancel", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
    expect(parseFeatureDirective("live:poll.cancel", "refresh")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
    expect(parseFeatureDirective("live:stream.visible", "orders")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
  });

  it("preserves generated feature conflict and repeated-modifier behavior", () => {
    expect(parseFeatureDirective("live:upload", "avatar", ["live:model.blur"])).toMatchObject({
      ok: false,
      code: "directive_conflict",
      fallback: "native",
    });
    expect(parseFeatureDirective("live:poll.visible.visible", "refresh")).toMatchObject({
      ok: false,
      code: "repeated_modifier",
      fallback: "inert",
    });
  });

  it("rejects endpoint-shaped progress values without weakening generic targets", () => {
    expect(parseFeatureDirective("live:progress", "/uploads/chunk")).toEqual({
      ok: false,
      code: "invalid_value",
      fallback: "inert",
    });
    expect(parseDirective("live:teleport", "/dialog")).toMatchObject({ ok: true });
  });

  it("rejects generated mutually exclusive feature modifiers", () => {
    for (const attributeName of [
      "live:stream.push-only.hybrid",
      "live:poll.visible.always",
      "live:poll.5s.30s",
      "live:poll.visible.always.5s.30s",
    ]) {
      expect(parseFeatureDirective(attributeName, "refresh")).toEqual({
        ok: false,
        code: "modifier_conflict",
        fallback: "inert",
      });
    }
  });

  it("uses one bounded lexical grammar for every promoted scalar value kind", () => {
    const invalid = ["-", "123abc", "Refresh", "9".repeat(65)] as const;
    for (const name of ["upload", "progress", "poll", "stream"] as const) {
      for (const value of invalid) {
        expect(parseFeatureDirective(`live:${name}`, value)).toMatchObject({
          ok: false,
          code: "invalid_value",
        });
      }
    }

    for (const name of ["upload", "progress", "poll", "stream"] as const) {
      expect(parseFeatureDirective(`live:${name}`, "registered_name")).toMatchObject({ ok: true });
    }
    for (const value of ["0", "-1", "9007199254740991"] as const) {
      expect(parseFeatureDirective("live:progress", value)).toMatchObject({ ok: true });
    }
    for (const value of ["01", "-0", "9007199254740992"] as const) {
      expect(parseFeatureDirective("live:progress", value)).toMatchObject({
        ok: false,
        code: "invalid_value",
      });
    }
  });
});
