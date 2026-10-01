import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("registered effects and public calls remain bounded to declared island ownership", async ({
  page,
}) => {
  const liveRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
  });
  const runtime = new RuntimePage(page);
  await runtime.open("effects");
  await expect(page.locator("html")).toHaveAttribute("data-extensions-ready", "true");
  await runtime.expectStatus("connected");

  await expect(page.locator("#effect-output")).toHaveText("effect-ready");
  await expect(page.locator("#extension-panel")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-effect-status", "completed");
  await expect(page.locator("html")).toHaveAttribute("data-call-result", "true");
  await expect(page.locator("html")).toHaveAttribute("data-wrong-scope", "invalid_context");
  await expect(page.locator("html")).toHaveAttribute("data-forged-call", "rejected");
  await expect(page.locator("html")).toHaveAttribute("data-effect-context", "call,island");
  await expect(page.locator("html")).toHaveAttribute("data-effect-mutation", "false:false");
  expect(liveRequests).toEqual([]);
});
