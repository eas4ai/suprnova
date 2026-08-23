import { describe, expect, it } from "vitest";

import { parseDirective } from "../src/directives/parser.js";
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

  it("parses every reviewed role and asynchronous modifier", () => {
    for (const role of ["cancel", "retry", "remove"] as const) {
      expect(parseDirective(`live:upload.${role}`, "avatar")).toEqual({
        ok: true,
        name: "upload",
        value: "avatar",
        role,
        modifiers: [],
      });
    }
    expect(parseDirective("live:progress", "avatar")).toMatchObject({
      ok: true,
      role: null,
    });
    expect(parseDirective("live:poll.visible.30s", "refresh")).toEqual({
      ok: true,
      name: "poll",
      value: "refresh",
      role: null,
      modifiers: ["visible", "30s"],
    });
    expect(parseDirective("live:stream.push-only", "orders")).toMatchObject({
      ok: true,
      role: null,
      modifiers: ["push-only"],
    });
  });

  it("rejects illegal roles, role conflicts, and unsupported modifiers with closed fallback", () => {
    expect(parseDirective("live:upload.stream", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "native",
    });
    expect(parseDirective("live:upload.cancel.retry", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "native",
    });
    expect(parseDirective("live:progress.cancel", "avatar")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
    expect(parseDirective("live:poll.cancel", "refresh")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
    expect(parseDirective("live:stream.visible", "orders")).toEqual({
      ok: false,
      code: "unsupported_modifier",
      fallback: "inert",
    });
  });

  it("preserves generated conflict and repeated-modifier behavior", () => {
    expect(parseDirective("live:upload", "avatar", ["live:model.blur"])).toMatchObject({
      ok: false,
      code: "directive_conflict",
      fallback: "native",
    });
    expect(parseDirective("live:poll.visible.visible", "refresh")).toMatchObject({
      ok: false,
      code: "repeated_modifier",
      fallback: "inert",
    });
  });
});
