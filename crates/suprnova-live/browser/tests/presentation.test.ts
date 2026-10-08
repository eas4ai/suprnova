import { describe, expect, it } from "vitest";

import {
  attributeProjection,
  isSafeAttributeName,
  isSafeClassName,
  presentationBoolean,
} from "../src/signals/presentation.js";

describe("local presentation contracts", () => {
  it("allows bounded presentation names and rejects executable or navigation surfaces", () => {
    expect(isSafeClassName("is-open")).toBe(true);
    expect(isSafeClassName("--runtime-code")).toBe(false);
    expect(isSafeAttributeName("aria-expanded")).toBe(true);
    expect(isSafeAttributeName("data-state")).toBe(true);
    for (const name of [
      "onclick",
      "style",
      "href",
      "src",
      "poster",
      "formaction",
      "type",
      "nonce",
      "data-controller",
      "data-menu-target",
      "data-suprnova-live-snapshot",
      "xlink-href",
    ]) {
      expect(isSafeAttributeName(name)).toBe(false);
    }
  });

  it("projects attributes without executable text or boolean ambiguity", () => {
    expect(attributeProjection("aria-expanded", true)).toEqual({ kind: "set", value: "true" });
    expect(attributeProjection("data-open", true)).toEqual({ kind: "set", value: "" });
    expect(attributeProjection("data-open", false)).toEqual({ kind: "remove" });
    expect(attributeProjection("title", "ready")).toEqual({ kind: "set", value: "ready" });
    expect(attributeProjection("title", null)).toEqual({ kind: "remove" });
    expect(() => attributeProjection("onclick", "alert(1)")).toThrow(
      "presentation_attribute_unsafe",
    );
  });

  it("requires booleans for semantic presentation directives", () => {
    expect(presentationBoolean(true)).toBe(true);
    expect(() => presentationBoolean("true")).toThrow("presentation_boolean_required");
  });
});
