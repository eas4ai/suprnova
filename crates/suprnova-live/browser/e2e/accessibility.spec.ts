import { expect, test } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

test("local disclosure and tab semantics remain keyboard-operable and accessible", async ({
  page,
}) => {
  await page.goto("/scenario/accessibility");
  await expectNoSeriousA11yViolations(page);

  const disclosure = page.locator("#a11y-disclosure");
  await disclosure.focus();
  await page.keyboard.press("Enter");
  await expect(disclosure).toHaveAttribute("aria-expanded", "true");
  await expect(page.locator("#a11y-panel")).toBeVisible();

  const secondTab = page.locator("#a11y-tab-second");
  await secondTab.focus();
  await page.keyboard.press("Enter");
  await expect(secondTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#a11y-tab-first")).toHaveAttribute("aria-selected", "false");
  await expect(page.locator("#a11y-tabpanel-first")).toBeHidden();
  await expect(page.locator("#a11y-tabpanel-second")).toBeVisible();
  await expectNoSeriousA11yViolations(page);
});

test("forms, errors, live feedback, busy state, and inert content expose explicit semantics", async ({
  page,
}) => {
  await page.goto("/scenario/accessibility");
  await expect(page.locator("#a11y-name")).toHaveAttribute("aria-invalid", "true");
  await expect(page.locator("#a11y-error")).toHaveAttribute("role", "alert");
  await expect(page.locator("#a11y-live")).toHaveAttribute("aria-live", "polite");
  await expect(page.locator("#a11y-busy")).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#a11y-disabled")).toBeDisabled();
  await expect(page.locator("#a11y-inert")).toHaveAttribute("inert", "");
  await expect(page.locator("#a11y-fallback")).toHaveAttribute("href", /navigationDestination/u);
  await page.locator("#a11y-fallback").click();
  await expect(page.locator("#destination-marker")).toHaveText("Complete canonical destination");
});

test("focus recovery, dirty guards, and reduced motion retain their explicit browser semantics", async ({
  page,
}) => {
  await page.goto("/scenario/navigation");
  page.once("dialog", async (dialog) => {
    await dialog.dismiss();
  });
  await page.locator("#dirty-input").fill("unsaved");
  await page.locator("#guarded-link").click();
  await expect(page.locator("#guarded-link")).toBeFocused();

  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/scenario/transitions");
  await page.locator("#transition-action").click();
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  await expect(page.locator("[data-suprnova-live-transition-state]")).toHaveCount(0);
});
