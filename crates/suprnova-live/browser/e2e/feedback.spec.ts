import { expect, test } from "@playwright/test";
import axe from "axe-core";

import { RuntimePage } from "./support/runtime-page.js";

test("feedback targets reflect authoritative work and remain keyboard safe", async ({ page }) => {
  await page.clock.install();
  const runtime = new RuntimePage(page);
  await runtime.open("feedback");
  await runtime.expectStatus("connected");

  const dirty = page.locator("#feedback-dirty");
  await expect(dirty).toBeHidden();
  await expect(page.locator("#feedback-combined")).toBeVisible();
  await page.locator("#feedback-model").fill("Grace");
  await expect(dirty).toBeVisible();

  await page.locator("#feedback-action").click();
  await expect(page.locator("#feedback-retrying")).toBeVisible();
  await expect(page.locator("#feedback-live")).toHaveText("Retrying");
  await expect(page.locator("#feedback-action")).toBeDisabled();
  await expect(page.locator("#feedback-busy")).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#feedback-combined")).toHaveClass(/\blive-loading\b/u);

  const escape = page.locator("#feedback-escape");
  await expect(escape).not.toHaveAttribute("disabled", "");
  await escape.press("Enter");
  await expect(page).toHaveURL(/#feedback-escaped$/u);

  await page.locator("#feedback-retrying").evaluate((element) => {
    element.remove();
  });
  await page.clock.fastForward(250);
  await expect(page.locator("#feedback-retrying")).toHaveCount(0);

  await page.addScriptTag({ content: axe.source });
  const violations = await page.evaluate(async () => {
    const accessibility = (
      globalThis as typeof globalThis & {
        axe: { run(root: Element): Promise<{ violations: readonly unknown[] }> };
      }
    ).axe;
    const island = document.querySelector("[data-suprnova-live-island]");
    if (island === null) throw new Error("missing_live_island");
    return (await accessibility.run(island)).violations;
  });
  expect(violations).toEqual([]);
});
