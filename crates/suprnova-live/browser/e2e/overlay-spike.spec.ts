import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

// The overlay family's spike (Cairn component-library-overlays): what each
// qualified engine offers for native open state, positioning and keyed
// continuity. The capability line is printed so the review can record it;
// the assertions hold the library to the agreed floor (popover and dialog)
// and to keyed continuity of an open popover and disclosure across a morph.
test("overlay spike: native primitives, anchor positioning and keyed continuity", async ({
  page,
  browserName,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("overlaySpike");
  await runtime.expectStatus("connected");

  const capabilities = await page.evaluate(() => ({
    popover: Object.prototype.hasOwnProperty.call(HTMLElement.prototype, "popover"),
    dialog: "showModal" in HTMLDialogElement.prototype,
    closedBy: "closedBy" in HTMLDialogElement.prototype,
    detailsName: "name" in HTMLDetailsElement.prototype,
    anchorPositioning:
      CSS.supports("anchor-name: --sn-spike") && CSS.supports("position-area: block-end"),
    commandFor: "commandForElement" in HTMLButtonElement.prototype,
  }));
  console.log(`overlay-spike ${browserName} ${JSON.stringify(capabilities)}`);
  test.info().annotations.push({
    type: "overlay-spike",
    description: `${browserName} ${JSON.stringify(capabilities)}`,
  });
  expect(capabilities.popover, "the popover attribute is at the floor").toBe(true);
  expect(capabilities.dialog, "the dialog element is at the floor").toBe(true);

  // Native open state, no script: the invoker opens the popover, the
  // dialog opens through showModal, Escape closes both.
  await page.locator("#spike-popover-trigger").click();
  await expect(page.locator("#spike-popover")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#spike-popover")).toBeHidden();

  // A keyed, preserved disclosure and popover keep their open state across
  // a morph that did not touch it, and the morph still reaches their content.
  await page.locator("#spike-notes > summary").click();
  await expect(page.locator("#spike-notes")).toHaveAttribute("open", "");
  await page.locator("#spike-popover-trigger").click();
  await expect(page.locator("#spike-popover")).toBeVisible();
  // A click outside a popover is its native light dismiss, so the morph is
  // invoked from inside it.
  await page.locator("#spike-popover-action").click();
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#spike-notes")).toHaveAttribute("open", "");
  await expect(page.locator("#spike-note-count")).toHaveText("2");
  await expect(page.locator("#spike-popover")).toBeVisible();
  await expect(page.locator("#spike-popover-text")).toHaveText("Hint 8");

  // The unkeyed disclosure follows the server, which renders it closed.
  await expect(page.locator("#spike-unkeyed")).not.toHaveAttribute("open", "");

  // The dialog contains focus and returns it to its invoker on close.
  await page.locator("#spike-dialog-trigger").click();
  await expect(page.locator("#spike-dialog")).toBeVisible();
  await expect(page.locator("#spike-dialog-close")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator("#spike-dialog")).toBeHidden();
  await expect(page.locator("#spike-dialog-trigger")).toBeFocused();
});
