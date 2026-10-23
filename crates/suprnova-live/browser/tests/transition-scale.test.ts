import { describe, expect, it } from "vitest";

import type { MorphIdentityEntry, MorphPlan } from "../src/morph/types.js";
import { prepareMorphTransitions } from "../src/transitions/lifecycle.js";

// A large sorted table reorders every keyed row in one morph. Deciding which
// entries moved once scanned the whole moved list for every entry, so 10,000
// moved rows cost about fifty million comparisons; a set makes it linear.

const ROWS = 10_000;
const BOUND_MS = 20;

function element(): Element {
  return { attributes: [], isEqualNode: () => true } as unknown as Element;
}

function reorderedPlan(): MorphPlan {
  const entries: MorphIdentityEntry[] = Array.from({ length: ROWS }, (_, index) =>
    Object.freeze({
      current: element(),
      currentPosition: `root/${String(index)}`,
      kind: "id" as const,
      replacement: element(),
      replacementPosition: `root/${String(ROWS - 1 - index)}`,
      token: `id:row-${String(index)}`,
      value: `row-${String(index)}`,
    }),
  );
  // Labels as the identity plan writes them: `#` and the id, built apart from
  // the entries, the way a parsed render produces them.
  const moved = entries.map((_, index) => `#row-${String(index)}`).reverse();
  return {
    identity: { entries, inserted: [], moved, removed: [] },
  } as unknown as MorphPlan;
}

describe("transitions over a large reorder", () => {
  it(`prepares a reorder of ${String(ROWS)} keyed rows in under ${String(BOUND_MS)} ms`, () => {
    const plan = reorderedPlan();
    prepareMorphTransitions(plan);
    const startedAt = performance.now();
    const transitions = prepareMorphTransitions(plan);
    const elapsed = performance.now() - startedAt;
    expect(transitions.before).toEqual([]);
    expect(elapsed).toBeLessThan(BOUND_MS);
  });
});
