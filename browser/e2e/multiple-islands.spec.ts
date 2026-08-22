import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("one saturated island cannot block another island scheduler", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("multipleSchedulers");
  await expect(page.locator("[data-suprnova-live-island]")).toHaveCount(2);

  for (let index = 0; index < 9; index += 1) await page.locator("#first-scheduler").click();
  await expect(page).toHaveURL(/#first-fallback$/u);
  await page.evaluate(() => {
    history.replaceState(null, "", location.pathname);
  });

  await page.locator("#second-scheduler").click();
  await expect(page).not.toHaveURL(/#second-fallback$/u);
  await expect(page.locator("#second-island")).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
});
