import { expect, test } from "@playwright/test";

import { RuntimePage, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

for (const scenario of ["cspNonce", "cspHash"] as const) {
  test(`${scenario} permits external runtime startup`, async ({ page }) => {
    const runtime = new RuntimePage(page);
    await runtime.open(scenario);
    await runtime.expectStatus("connected");
  });
}

test("blocked runtime leaves initial SSR content exposed", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("cspBlocked");
  await runtime.expectVisibleContent("Server-rendered search results");
  await expect(runtime.island()).not.toHaveAttribute(STATUS_ATTRIBUTE, /.+/u);
});
