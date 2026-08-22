import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("immediate and debounced models enter only their owning island scheduler", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("modelsImmediate");
  await page.locator("#immediate-model").fill("rust");
  await page.locator("#immediate-after").click();
  await expect(page).toHaveURL(/#immediate-fallback$/u);

  await runtime.open("modelsDebounce");
  await page.locator("#debounced-model").fill("r");
  await page.locator("#debounced-model").fill("rust");
  await page.waitForTimeout(150);
  await page.locator("#debounced-after").click();
  await expect(page).toHaveURL(/#debounced-fallback$/u);
});

test("submit samples controls once, preserves reset semantics, and excludes files and disabled controls", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("modelsForm");
  await page.locator("#model-query").fill("newer");
  await page.locator("#model-number").fill("7");
  await page.locator("#model-checkbox").check();
  await page.locator("#model-tags").selectOption(["rust", "zig"]);
  await page.locator("#model-file").setInputFiles({
    buffer: Buffer.from("browser-owned upload"),
    mimeType: "text/plain",
    name: "upload.txt",
  });
  await page.locator("#model-reset").click();
  await expect(page.locator("#model-query")).toHaveValue("initial");
  await expect(page.locator("#model-number")).toHaveValue("2");
  await expect(page.locator("#model-checkbox")).not.toBeChecked();

  await page.locator("#model-query").fill("submitted");
  await page.locator("#model-submit").click();
  await expect(page).not.toHaveURL(/\/models-native(?:\?|$)/u);
  await page.locator("#model-submit").click();
  await expect(page).toHaveURL(/\/models-native(?:\?|$)/u);
});

test("a nested island model cannot consume its parent scheduler capacity", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("modelsNested");
  await page.locator("#child-model").fill("child edit");
  await page.locator("#parent-after-child").click();
  await expect(page).not.toHaveURL(/#parent-fallback$/u);
});
