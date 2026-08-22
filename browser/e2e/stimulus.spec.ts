import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("the optional bridge preserves standard Stimulus lifecycle without bundling it", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("stimulus");

  await expect(page.locator("html")).toHaveAttribute("data-stimulus-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-disposal", "complete");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-preserved", "1:1");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-removed", "1:1");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-inserted", "2:2");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-detached", "2:2");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-nested", "1:1");
  await expect(page.locator("html")).toHaveAttribute("data-stimulus-errors", "1");
  await expect(page.locator("html")).toHaveAttribute(
    "data-stimulus-runtime-after-error",
    "connected",
  );
});
