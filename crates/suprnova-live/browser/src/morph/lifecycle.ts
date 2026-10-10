import type { MorphPlan, MorphResult } from "./types.js";

function logicalReplacements(plan: MorphPlan): readonly string[] {
  return plan.controls.bindings
    .filter(
      ({ control, current, replacement }) =>
        control.kind === "replace" && current !== null && replacement !== null,
    )
    .map(({ control }) => control.key);
}

function withLogicalReplacements(
  identities: readonly string[],
  replacements: readonly string[],
): readonly string[] {
  return Object.freeze([...new Set([...identities, ...replacements])]);
}

export function morphLifecycleResult(plan: MorphPlan): MorphResult {
  const replacements = logicalReplacements(plan);
  return Object.freeze({
    inserted: withLogicalReplacements(plan.identity.inserted, replacements),
    moved: plan.identity.moved,
    removed: withLogicalReplacements(plan.identity.removed, replacements),
    root: plan.currentRoot,
  });
}
