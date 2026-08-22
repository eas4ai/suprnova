import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("morph controls preserve their distinct identities and lifecycle across repeated updates", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  let liveRequests = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && new URL(request.url()).pathname === "/live") {
      liveRequests += 1;
    }
  });
  await runtime.open("preservation");
  await expect(page.locator("html")).toHaveAttribute("data-replace-lifecycle", "1:0");

  await page.evaluate(() => {
    const ignoredChild = document.querySelector("#ignored-child");
    const ignoredSubtree = document.querySelector("#ignored-subtree-child");
    if (ignoredChild !== null) ignoredChild.textContent = "Browser-owned child";
    if (ignoredSubtree !== null) ignoredSubtree.textContent = "Browser-owned subtree";
    Reflect.set(window, "__suprnovaPreservation", {
      persist: document.querySelector("#persisted-panel"),
      replace: document.querySelector("#replaced-panel"),
      teleport: document.querySelector("#teleported-dialog"),
    });
    const focus = document.querySelector("#teleported-focus");
    if (focus instanceof HTMLElement) focus.focus();
    const action = document.querySelector("#preservation-action");
    if (action instanceof HTMLElement) action.click();
  });

  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#preserved-panel")).toHaveAttribute("data-owner", "browser");
  await expect(page.locator("#preserved-child")).toHaveText("Server child 8");
  await expect(page.locator("#ignored-children")).toHaveAttribute("data-state", "server-8");
  await expect(page.locator("#ignored-child")).toHaveText("Browser-owned child");
  await expect(page.locator("#ignored-subtree")).toHaveAttribute("data-state", "browser");
  await expect(page.locator("#ignored-subtree-child")).toHaveText("Browser-owned subtree");
  await expect(page.locator("#persist-destination > #persisted-panel")).toHaveCount(1);
  await expect(page.locator("#teleported-focus")).toBeFocused();
  await page.locator("#persisted-toggle").click();
  await expect(page.locator("#persisted-state")).toBeVisible();
  await page.locator("#persisted-toggle").click();
  await expect(page.locator("#persisted-state")).toBeHidden();
  await expect(page.locator("#modal-root > #teleported-dialog")).toHaveCount(1);
  await expect(page.locator("#teleported-dialog")).toHaveAttribute(
    "aria-labelledby",
    "teleported-title",
  );
  await expect(page.locator("#teleported-title")).toHaveText("Dialog 8");
  await expect(page.locator("html")).toHaveAttribute("data-replace-lifecycle", "2:1");
  expect(
    await page.evaluate(() => {
      const initial = Reflect.get(window, "__suprnovaPreservation") as {
        readonly persist: Element | null;
        readonly replace: Element | null;
        readonly teleport: Element | null;
      };
      return {
        persist: initial.persist === document.querySelector("#persisted-panel"),
        replace: initial.replace !== document.querySelector("#replaced-panel"),
        teleport: initial.teleport === document.querySelector("#teleported-dialog"),
      };
    }),
  ).toEqual({ persist: true, replace: true, teleport: true });

  await page.locator("#preservation-action").click();
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "9");
  await expect(page.locator("#preserved-panel")).toHaveCount(0);
  await expect(page.locator("#persist-origin > #persisted-panel")).toHaveCount(1);
  await page.locator("#persisted-toggle").click();
  await expect(page.locator("#persisted-state")).toBeVisible();
  await expect(page.locator("#modal-root > #teleported-dialog")).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-replace-lifecycle", "3:2");
  await expect(page.locator("#ignored-child")).toHaveText("Browser-owned child");
  await expect(page.locator("#ignored-subtree-child")).toHaveText("Browser-owned subtree");

  await page.evaluate(() => {
    const ignored = document.querySelector("#ignored-subtree-child");
    if (ignored === null) throw new Error("ignored subtree missing");
    ignored.insertAdjacentHTML(
      "beforeend",
      '<button id="forged-action" live:click="forged" live:effect="forged">Forged</button>' +
        '<section id="forged-island" data-suprnova-live-island data-suprnova-live-component="forged"></section>' +
        '<div id="forged-target" live:teleport="#late-target">Forged teleport</div>',
    );
    document.body.insertAdjacentHTML("beforeend", '<div id="late-target"></div>');
  });
  await page.locator("#forged-action").click();
  await page.waitForTimeout(100);
  expect(liveRequests).toBe(2);
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "9");
  await expect(page.locator("#forged-island")).not.toHaveAttribute(
    "data-suprnova-live-status",
    /.+/u,
  );
  await expect(page.locator("#forged-target")).toHaveCount(1);
  await expect(page.locator("#late-target > #forged-target")).toHaveCount(0);
});

test("a target added after boot cannot acquire teleport authority", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("teleportLateTarget");
  await page.evaluate(() => {
    document.body.insertAdjacentHTML("beforeend", '<div id="late-modal-root"></div>');
  });

  await page.locator("#late-teleport-action").click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-morph-recovery",
    "/teleport-target-rejected",
  );
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "7");
  await expect(page.locator("#late-modal-root")).toBeEmpty();
});
