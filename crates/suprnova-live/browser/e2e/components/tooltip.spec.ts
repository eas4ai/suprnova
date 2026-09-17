import { expect, test, type Page } from "@playwright/test";

import { mountComponents } from "./support.js";

// The shipped tooltip in each engine: the bubble shows on hover and on
// keyboard focus in a document the enhancement never reaches (OVL-002), and
// Escape dismisses it where it is shown, with neither the pointer nor focus
// moved (OVL-008). The markup is what suprnova.tooltip renders.
const TOOLTIP = `<main style="padding: 4rem"><span class="sn-tooltip"><button class="sn-button" id="trigger" type="button" aria-describedby="tip">Save</button><span class="sn-tooltip-bubble" id="tip" role="tooltip">Saves the notes to the server</span></span><button class="sn-button" id="elsewhere" type="button">Elsewhere</button></main>`;

async function hoverTrigger(page: Page): Promise<void> {
  const box = await page.locator("#trigger").boundingBox();
  if (box === null) throw new Error("trigger has no box");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
}

test("OVL-002: the bubble shows on hover and on focus with no script in the document", async ({
  page,
}) => {
  await mountComponents(page, { components: ["tooltip"], html: TOOLTIP, scripts: false });
  await hoverTrigger(page);
  await expect(page.locator("#tip")).toBeVisible();
  await page.mouse.move(1, 1);
  await expect(page.locator("#tip")).toBeHidden();
  await page.keyboard.press("Tab");
  await expect(page.locator("#tip")).toBeVisible();
});

test("OVL-008: Escape dismisses the hovered bubble without moving the pointer, and the next hover shows it again", async ({
  page,
}) => {
  await mountComponents(page, { html: TOOLTIP, components: ["tooltip"] });
  await hoverTrigger(page);
  await expect(page.locator("#tip")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#tip")).toBeHidden();

  // The pointer never moved; the next hover of the same trigger shows it.
  await page.mouse.move(1, 1);
  await hoverTrigger(page);
  await expect(page.locator("#tip")).toBeVisible();
});

test("OVL-008: Escape dismisses the focused bubble, and focusing the trigger again shows it", async ({
  page,
}) => {
  await mountComponents(page, { html: TOOLTIP, components: ["tooltip"] });
  await page.keyboard.press("Tab");
  await expect(page.locator("#trigger")).toBeFocused();
  await expect(page.locator("#tip")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#tip")).toBeHidden();
  await expect(page.locator("#trigger")).toBeFocused();

  await page.locator("#elsewhere").focus();
  await page.locator("#trigger").focus();
  await expect(page.locator("#tip")).toBeVisible();
});
