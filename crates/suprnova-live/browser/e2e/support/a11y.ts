import { createRequire } from "node:module";

import { expect, type Page } from "@playwright/test";

const require = createRequire(import.meta.url);
const AXE_PATH = require.resolve("axe-core/axe.min.js");

interface ClosedViolation {
  readonly id: string;
  readonly impact: string | null;
  readonly nodes: number;
}

function closedViolations(value: unknown): readonly ClosedViolation[] {
  if (typeof value !== "object" || value === null) throw new Error("axe_result_invalid");
  const raw: unknown = Reflect.get(value, "violations");
  if (!Array.isArray(raw)) throw new Error("axe_violations_invalid");
  return Object.freeze(
    (raw as unknown[]).map((violation) => {
      if (typeof violation !== "object" || violation === null) {
        throw new Error("axe_violation_invalid");
      }
      const id: unknown = Reflect.get(violation, "id");
      const impact: unknown = Reflect.get(violation, "impact");
      const nodes: unknown = Reflect.get(violation, "nodes");
      if (typeof id !== "string" || (impact !== null && typeof impact !== "string")) {
        throw new Error("axe_violation_invalid");
      }
      return Object.freeze({
        id,
        impact,
        nodes: Array.isArray(nodes) ? nodes.length : 0,
      });
    }),
  );
}

export async function expectNoSeriousA11yViolations(
  page: Page,
  options: Readonly<{ sourceUrl?: string }> = {},
): Promise<void> {
  if (options.sourceUrl === undefined) await page.addScriptTag({ path: AXE_PATH });
  else await page.addScriptTag({ url: options.sourceUrl });
  const result: unknown = await page.evaluate<unknown>(async () => {
    const axe: unknown = Reflect.get(window, "axe");
    if (typeof axe !== "object" || axe === null) throw new Error("axe_unavailable");
    const run: unknown = Reflect.get(axe, "run");
    if (typeof run !== "function") throw new Error("axe_run_unavailable");
    const value: unknown = await Reflect.apply(run, axe, [
      document,
      { resultTypes: ["violations"] },
    ]);
    return value;
  });
  const serious = closedViolations(result).filter(
    (violation) => violation.impact === "critical" || violation.impact === "serious",
  );
  expect(serious).toEqual([]);
}
