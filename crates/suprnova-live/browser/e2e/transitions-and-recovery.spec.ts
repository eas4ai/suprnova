import { expect, test } from "@playwright/test";

async function transitionTrace(page: import("@playwright/test").Page): Promise<readonly string[]> {
  return page.evaluate(() => {
    const value: unknown = Reflect.get(window, "__suprnovaTransitionTrace");
    return Array.isArray(value) ? value.map((entry: unknown) => String(entry)) : [];
  });
}

test("enter, leave, move, and state transitions settle before accepted authority commits", async ({
  page,
}) => {
  await page.goto("/scenario/transitions");
  const island = page.locator('[data-suprnova-live-document-key="primary"]');

  await page.locator("#transition-action").click();
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#transition-enter")).toHaveText("Enter");
  await expect(page.locator("#transition-state")).toHaveText("After");
  await expect(page.locator("#transition-leave")).toHaveCount(0);
  await expect
    .poll(() => transitionTrace(page))
    .toEqual(
      expect.arrayContaining([
        "transition-leave:leave:fade",
        "transition-enter:enter:fade",
        "transition-move:move:fade",
        "transition-state:state:fade",
      ]),
    );
  await expect(page.locator("[data-suprnova-live-transition-state]")).toHaveCount(0);
});

test("reduced motion and a missing animation API preserve the same semantic result", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/scenario/transitions");
  await page.locator("#transition-action").click();
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  expect(await transitionTrace(page)).toEqual([]);

  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/scenario/transitionsUnsupported");
  await page.locator("#transition-action").click();
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  expect(await transitionTrace(page)).toEqual([]);
});

test("a failed fresh render disconnects only its island without replay or document reload", async ({
  page,
}) => {
  await page.goto("/scenario/recoveryFails");
  const island = page.locator('[data-suprnova-live-document-key="primary"]');
  await page.locator("#recovery-action").click();

  await expect(island).toHaveAttribute("data-suprnova-live-status", "disconnected");
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "7");
  await expect(page.locator("#recovery-content")).toHaveText("Last accepted");
  await expect(page.locator("#recovery-corrupt")).toHaveCount(0);
  await expect(page.locator("html")).not.toHaveAttribute("data-recovery-script-executed", "true");
  await expect(page.locator("#recovery-action")).toHaveCount(1);
});
