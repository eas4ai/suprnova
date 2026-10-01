import { readFile } from "node:fs/promises";

import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

for (const scenario of [
  "cspModuleNonce",
  "cspModuleHash",
  "cspClassicNonce",
  "cspClassicHash",
] as const) {
  test(`${scenario} boots only external checked assets`, async ({ page }) => {
    const runtime = new RuntimePage(page);
    await runtime.open(scenario);
    await runtime.expectStatus("connected");
    await expect(page.locator('script:not([src]):not([type="application/json"])')).toHaveCount(0);
  });
}

test("production artifacts contain no runtime code generation or dynamic module URL", async () => {
  const directory = new URL("../dist/", import.meta.url);
  for (const file of ["suprnova-live.esm.js", "suprnova-live.classic.js"]) {
    const source = await readFile(new URL(file, directory), "utf8");
    expect(source).not.toMatch(/\beval\s*\(/u);
    expect(source).not.toMatch(/\bnew\s+Function\b/u);
    expect(source).not.toMatch(/\bimport\s*\(/u);
    expect(source).not.toContain('diagnostics:"verbose"');
  }
});

test("published declarations include the suspended lifecycle state", async () => {
  const declarations = await readFile(new URL("../dist/index.d.ts", import.meta.url), "utf8");
  expect(declarations).toContain(
    'export type RuntimeStatus = "running" | "suspended" | "stopped";',
  );
});
