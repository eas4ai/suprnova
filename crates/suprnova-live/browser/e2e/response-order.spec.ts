import { expect, test } from "@playwright/test";

for (const scenario of ["responseRedirect", "responseNavigated"] as const) {
  test(`${scenario} navigates before any in-page response effect`, async ({ page }) => {
    await page.goto(`/scenario/${scenario}`);
    const island = page.locator("[data-suprnova-live-island]");
    await expect(island).toHaveAttribute("data-suprnova-live-status", "connected");
    await page.locator("#response-action").click();
    await expect
      .poll(() =>
        page.evaluate(() => document.documentElement.getAttribute("data-navigation-target")),
      )
      .toBe(
        scenario === "responseRedirect"
          ? "/transport-accepted"
          : "/response-order-target?kind=navigated",
      );
    expect(
      await page.evaluate(() => {
        const value: unknown = Reflect.get(window, "__suprnovaResponseTrace");
        return Array.isArray(value) ? value.map((entry: unknown) => String(entry)) : [];
      }),
    ).toEqual(["navigate"]);
    await expect(page.locator("#response-content")).toHaveText("Original");
    await expect(island).toHaveAttribute("data-suprnova-live-revision", "7");
  });
}

test("committed HTML applies the complete post-morph order", async ({ page }) => {
  await page.goto("/scenario/responseCommitted");
  const island = page.locator("[data-suprnova-live-island]");
  const priorSnapshot = await island.getAttribute("data-suprnova-live-snapshot");
  await page.locator("#response-action").click();
  await expect(page.locator("#response-content")).toHaveText("Updated");
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect
    .poll(async () => island.getAttribute("data-suprnova-live-snapshot"))
    .not.toBe(priorSnapshot);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const value: unknown = Reflect.get(window, "__suprnovaResponseTrace");
        return Array.isArray(value) ? value.map((entry: unknown) => String(entry)) : [];
      }),
    )
    .toEqual(["replace", "event", "effect"]);
});

test("no-render commits successor authority without replacing content", async ({ page }) => {
  await page.goto("/scenario/responseNoRender");
  const island = page.locator("[data-suprnova-live-island]");
  const priorSnapshot = await island.getAttribute("data-suprnova-live-snapshot");
  await page.locator("#response-action").click();
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#response-content")).toHaveText("Original");
  await expect
    .poll(async () => island.getAttribute("data-suprnova-live-snapshot"))
    .not.toBe(priorSnapshot);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const value: unknown = Reflect.get(window, "__suprnovaResponseTrace");
        return Array.isArray(value) ? value.map((entry: unknown) => String(entry)) : [];
      }),
    )
    .toEqual(["event", "effect"]);
});
