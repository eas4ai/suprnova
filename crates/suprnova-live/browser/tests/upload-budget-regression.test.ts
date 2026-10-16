import { describe, expect, it } from "vitest";

import { regressionAtLeast15Percent } from "../benchmarks/upload-schema.js";

describe("U4/16 regression threshold", () => {
  it("allows just below 15 percent and rejects exact or greater growth", () => {
    expect(regressionAtLeast15Percent(1.1499, 1)).toBe(false);
    expect(regressionAtLeast15Percent(1.15, 1)).toBe(true);
    expect(regressionAtLeast15Percent(1.1501, 1)).toBe(true);
  });

  it("uses a stable integer comparison for decimal measurements", () => {
    expect(regressionAtLeast15Percent(0.114_999_999, 0.1)).toBe(false);
    expect(regressionAtLeast15Percent(0.115, 0.1)).toBe(true);
  });
});
