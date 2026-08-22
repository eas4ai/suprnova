import { expect, test } from "@playwright/test";

test("checked cross-document names enhance navigation without changing its semantics", async ({
  page,
}) => {
  await page.goto("/scenario/documentTransition");
  const supported = await page.evaluate(
    () =>
      typeof document.startViewTransition === "function" &&
      typeof Reflect.get(window, "PageRevealEvent") === "function" &&
      CSS.supports("view-transition-name", "none") &&
      !matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  const name = await page
    .locator("#transition-hero")
    .evaluate((element) =>
      getComputedStyle(element).getPropertyValue("view-transition-name").trim(),
    );
  expect(name).toBe(supported ? "suprnova-document-hero" : "none");

  await page.locator("#document-transition-link").click();
  await expect(page).toHaveURL(/documentTransitionDestination$/u);
  await expect(page.locator("#transition-destination-focus")).toBeFocused();
  await expect(page.locator("#transition-hero")).toHaveText("Hero destination");
});

test("reduced motion, unsupported support, and capture failure fall back identically", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/scenario/documentTransition");
  await expect(page.locator("#transition-hero")).toHaveCSS("view-transition-name", "none");
  await page.locator("#document-transition-link").click();
  await expect(page.locator("#transition-destination-focus")).toBeFocused();

  await page.emulateMedia({ reducedMotion: "no-preference" });
  for (const scenario of ["documentTransitionUnsupported", "documentTransitionCaptureFailure"]) {
    await page.goto(`/scenario/${scenario}`);
    await page.locator("#document-transition-link").click();
    await expect(page.locator("#transition-destination-focus")).toBeFocused();
  }
});

test("transition cancellation through the dirty guard keeps the old document interactive", async ({
  page,
}) => {
  await page.goto("/scenario/documentTransition");
  await page.locator("#dirty-input").fill("unsaved");
  page.once("dialog", (dialog) => dialog.dismiss());
  await page.locator("#cancel-transition-link").click();
  await expect(page).toHaveURL(/\/scenario\/documentTransition$/u);
  await expect(page.locator("#cancel-transition-link")).toBeFocused();
  await expect(page.locator("#document-transition-link")).toBeEnabled();
});
