import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

for (const scenario of ["seedAction", "seedActionNoCrypto"] as const) {
  test(`${scenario} promotes only when transport identity is available`, async ({ page }) => {
    const liveRequests: string[] = [];
    page.on("request", (request) => {
      if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
    });
    const runtime = new RuntimePage(page);
    await runtime.open(scenario);
    await runtime.expectStatus("connected");

    const routed = await page
      .locator("#seed-action")
      .evaluate((element) =>
        element.dispatchEvent(
          new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
        ),
      );
    expect(routed).toBe(scenario === "seedActionNoCrypto");
    if (scenario === "seedAction") await expect.poll(() => liveRequests.length).toBe(1);
    else expect(liveRequests).toEqual([]);
  });
}

test("repeated lazy discovery remains connected and makes no eager request", async ({ page }) => {
  const liveRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
  });
  const runtime = new RuntimePage(page);
  await runtime.open("lazySeed");
  await runtime.expectStatus("connected");
  await page.evaluate(() => {
    const root = document.querySelector("[data-suprnova-live-island]");
    root?.parentNode?.insertBefore(document.createTextNode("scan"), root.nextSibling);
    root?.parentNode?.insertBefore(document.createTextNode("again"), root.nextSibling);
  });
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-status", "connected");
  expect(liveRequests).toEqual([]);
});
