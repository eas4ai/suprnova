import { expect, test } from "@playwright/test";

import { mountComponents } from "./support.js";

// The markup suprnova.date_picker renders, cut to one strip. The base layer
// draws every `details` as a bordered surface for the collapsible; the date
// picker's own disclosure must not inherit it, or "Pick the parts" reads as a
// second, empty field under the date input (FORM-007).
const PICKER = `<main style="padding: 1.5rem"><sn-date-picker class="sn-date"><label class="sn-date-label" for="when">Date</label><input class="sn-input sn-date-input" id="when" name="when" type="date" min="2026-01-01" max="2028-12-31"><details class="sn-date-strips"><summary class="sn-date-summary">Pick the parts</summary><fieldset class="sn-date-strip" data-sn-part="year"><legend class="sn-date-legend">Year</legend><label class="sn-date-option"><input class="sn-date-radio" type="radio" name="when-year" value="2026">2026</label></fieldset></details></sn-date-picker></main>`;

for (const theme of ["light", "dark"] as const) {
  test(`FORM-007: the date picker's disclosure is a plain toggle, not a second field, in the ${theme} scheme`, async ({
    page,
  }) => {
    await mountComponents(page, { html: PICKER, components: ["date-picker"], theme });
    const disclosure = await page.locator(".sn-date-strips").evaluate((details) => {
      const style = getComputedStyle(details);
      return { border: style.borderTopStyle, background: style.backgroundColor };
    });
    expect(disclosure).toEqual({ border: "none", background: "rgba(0, 0, 0, 0)" });

    await page.locator(".sn-date-summary").click();
    const strip = await page
      .locator(".sn-date-strip")
      .evaluate((fieldset) => getComputedStyle(fieldset).borderTopStyle);
    expect(strip).toBe("solid");
  });
}
