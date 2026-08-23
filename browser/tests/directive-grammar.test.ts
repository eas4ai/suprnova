import { describe, expect, it } from "vitest";

import { expectedFixtureManifestSha256, loadFixtureSet } from "../src/conformance.js";
import {
  DIRECTIVE_CONTRACTS,
  DIRECTIVE_ARGUMENT_FORMS,
  DIRECTIVE_EVENT_TYPES,
  DIRECTIVE_FALLBACKS,
  DIRECTIVE_FIXTURE_MANIFEST_SHA256,
  DIRECTIVE_LITERAL_KINDS,
  DIRECTIVE_TARGET_KINDS,
  RESERVED_DIRECTIVES,
} from "../src/generated/directive-contract.js";
import { parseDirective } from "../src/directives/parser.js";
import { asRecord, asString } from "../src/schema.js";

const EXPECTED_NAMES = [
  "click",
  "submit",
  "change",
  "input",
  "keydown",
  "init",
  "model",
  "url",
  "signal",
  "toggle",
  "show",
  "class",
  "attr",
  "selected",
  "expanded",
  "inert",
  "focus",
  "idle",
  "dirty",
  "queued",
  "loading",
  "validating",
  "success",
  "interrupted",
  "offline",
  "retrying",
  "error",
  "effect",
  "on",
  "call",
  "component",
  "key",
  "lazy",
  "preserve",
  "ignore",
  "replace",
  "persist",
  "teleport",
  "transition",
  "navigate",
  "prefetch",
  "upload",
  "progress",
  "poll",
  "stream",
] as const;

function sampleValue(value: string): string {
  switch (value) {
    case "empty":
      return "";
    case "mapping":
      return "open:false";
    case "target":
      return "save";
    default:
      return "query";
  }
}

function stringArray(value: unknown): readonly string[] {
  if (!Array.isArray(value) || !value.every((entry) => typeof entry === "string")) {
    throw new TypeError("expected_string_array");
  }
  return value;
}

function fixtureModifiers(grammar: Record<string, unknown>, entry: Record<string, unknown>) {
  const modifiers = entry["modifiers"];
  return typeof modifiers === "string"
    ? stringArray(grammar[`${modifiers}_modifiers`])
    : stringArray(modifiers);
}

describe("closed directive grammar", () => {
  it("matches the reviewed v4 fixture and its manifest identity", async () => {
    const fixtures = await loadFixtureSet(4);
    const grammar = asRecord(fixtures.get("directive-grammar.json"));
    const entries = grammar["directives"] as unknown[];
    const fixtureNames = entries.map((entry) => asString(asRecord(entry)["name"]));
    const syntax = asRecord(grammar["syntax"]);

    expect(DIRECTIVE_CONTRACTS.map(({ name }) => name)).toEqual(EXPECTED_NAMES);
    expect(fixtureNames).toEqual(EXPECTED_NAMES);
    expect(DIRECTIVE_TARGET_KINDS).toEqual(stringArray(syntax["target_kinds"]));
    expect(DIRECTIVE_LITERAL_KINDS).toEqual(stringArray(syntax["literal_kinds"]));
    expect(DIRECTIVE_ARGUMENT_FORMS).toEqual(stringArray(syntax["argument_forms"]));
    expect(DIRECTIVE_FALLBACKS).toEqual(stringArray(syntax["fallbacks"]));
    expect(DIRECTIVE_EVENT_TYPES).toEqual(["click", "submit", "change", "input", "keydown"]);
    for (const [index, contract] of DIRECTIVE_CONTRACTS.entries()) {
      const entry = asRecord(entries[index]);
      expect(contract).toEqual({
        name: asString(entry["name"]),
        owner: asString(entry["owner"]),
        value: asString(entry["value"]),
        modifiers: fixtureModifiers(grammar, entry),
        roles: stringArray(entry["roles"]),
        conflicts: stringArray(entry["conflicts"]),
        phase: asString(entry["phase"]),
        fallback: asString(entry["fallback"]),
        capability: entry["capability"] === null ? null : asString(entry["capability"]),
      });
    }
    expect(RESERVED_DIRECTIVES).toEqual([]);
    expect(DIRECTIVE_FIXTURE_MANIFEST_SHA256).toBe(await expectedFixtureManifestSha256(4));
  });

  it("parses every directive and every enumerated modifier without evaluating values", () => {
    for (const contract of DIRECTIVE_CONTRACTS) {
      const value = sampleValue(contract.value);
      expect(parseDirective(`live:${contract.name}`, value)).toMatchObject({
        ok: true,
        name: contract.name,
      });
      for (const modifier of contract.modifiers) {
        expect(parseDirective(`live:${contract.name}.${modifier}`, value)).toMatchObject({
          ok: true,
          name: contract.name,
          modifiers: [modifier],
        });
      }
      for (const role of contract.roles) {
        expect(parseDirective(`live:${contract.name}.${role}`, value)).toMatchObject({
          ok: true,
          name: contract.name,
          role,
          modifiers: [],
        });
      }
      for (const conflict of contract.conflicts) {
        expect(parseDirective(`live:${contract.name}`, value, [conflict])).toMatchObject({
          ok: false,
          code: "directive_conflict",
          fallback: contract.fallback,
        });
      }
    }
  });

  it("fails closed on reserved, unknown, repeated, dynamic, unsafe, and conflicting forms", () => {
    expect(parseDirective("live:nope", "save")).toMatchObject({
      ok: false,
      code: "unknown_directive",
    });
    expect(parseDirective("live:click.prevent.prevent", "save")).toMatchObject({
      ok: false,
      code: "repeated_modifier",
    });
    expect(parseDirective("live:model.debounce.999ms", "query")).toMatchObject({
      ok: false,
      code: "invalid_modifier",
    });
    expect(parseDirective("live:{{directive}}", "save")).toMatchObject({
      ok: false,
      code: "dynamic_structure_unproved",
    });
    expect(parseDirective("live:teleport", "//evil.example")).toMatchObject({
      ok: false,
      code: "unsafe_target",
    });
    expect(parseDirective("live:teleport", "/unsafe\\target")).toMatchObject({
      ok: false,
      code: "unsafe_target",
    });
    expect(parseDirective("live:preserve", "", ["replace"])).toMatchObject({
      ok: false,
      code: "directive_conflict",
    });
    expect(parseDirective("live:preserve", "", ["x".repeat(257)])).toMatchObject({
      ok: false,
      code: "attribute_limit",
    });
  });
});
