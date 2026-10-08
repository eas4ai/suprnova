import { expect, test } from "@playwright/test";
import axe from "axe-core";

import { RuntimePage } from "./support/runtime-page.js";

test("local signals update accessible presentation without a server request", async ({ page }) => {
  const liveRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
  });
  await page.emulateMedia({ reducedMotion: "reduce" });
  const runtime = new RuntimePage(page);
  await runtime.open("localSignals");
  await runtime.expectStatus("connected");

  const panel = page.locator("#signal-panel");
  await expect(panel).toBeHidden();
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator("#signal-mismatch")).toBeHidden();
  await expect(page.locator("#child-panel")).toBeVisible();
  await expect(page.locator("#unsafe-local")).not.toHaveAttribute("onclick", /.+/u);
  await page.locator("#signal-toggle").press("Enter");
  await expect(panel).toBeVisible();
  await expect(panel).not.toHaveAttribute("hidden", "");
  await expect(panel).not.toHaveAttribute("inert", "");
  await expect(panel).not.toHaveAttribute("aria-hidden", "true");
  await expect(panel).toHaveClass(/\bis-open\b/u);
  await expect(panel).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#signal-tab")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#signal-disclosure")).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#signal-guard")).toHaveAttribute("inert", "");
  await expect(page.locator("#signal-combined")).toBeVisible();
  await expect(page.locator("#signal-combined")).toHaveAttribute("inert", "");
  await expect(page.locator("#signal-focus")).toBeFocused();
  await expect(page.locator("#child-panel")).toBeVisible();
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
  expect(liveRequests).toEqual([]);
});

test("third-party attribute mutation and reinsertion cannot create local directive authority", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("localSignals");
  await runtime.expectStatus("connected");
  const result = await page.evaluate(async () => {
    const late = document.querySelector("#late-local");
    late?.setAttribute("live:show", "open");
    document
      .querySelector("#signal-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }));
    const attributeOnly = late?.hasAttribute("hidden") ?? false;
    late?.remove();
    if (late === null) throw new Error("missing_late_local");
    document.querySelector("[data-suprnova-live-island]")?.append(late);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        resolve();
      }),
    );
    document
      .querySelector("#signal-toggle")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }));
    await Promise.resolve();
    return { attributeOnly, revalidated: late.hasAttribute("hidden") };
  });
  expect(result).toEqual({ attributeOnly: false, revalidated: false });
});
