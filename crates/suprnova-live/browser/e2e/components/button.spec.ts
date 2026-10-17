import { expect, test, type Page } from "@playwright/test";

import { mountComponents } from "./support.js";

// What suprnova.button renders for each variant. A click button carries
// type="button", which the base layer paints as a secondary control so bare
// HTML reads right; the macro's default variant is primary, so the button
// stylesheet must paint data-sn-variant="primary" as the primary action.
const BUTTONS = `<main style="padding: 1.5rem"><button class="sn-button" id="primary" type="button" data-sn-variant="primary">Save</button> <button class="sn-button" id="submit" type="submit" data-sn-variant="primary">Submit</button> <button class="sn-button" id="secondary" type="button" data-sn-variant="secondary">Cancel</button> <a class="sn-button" id="link" role="button" href="#" data-sn-variant="primary">Open</a> <button class="sn-button" id="pressed" type="button" data-sn-variant="primary" aria-pressed="true">Bold</button></main>`;

interface Paint {
  readonly background: string;
  readonly border: string;
  readonly color: string;
}

async function paintOf(page: Page, id: string): Promise<Paint> {
  return page.locator(`#${id}`).evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      background: style.backgroundColor,
      border: style.borderTopColor,
      color: style.color,
    };
  });
}

// The colors the tokens resolve to in this document, read through a probe so
// the comparison holds in either scheme and in every engine's color syntax.
async function tokenPaint(page: Page, background: string, text: string): Promise<Paint> {
  return page.evaluate(
    ({ background, text }) => {
      const probe = document.createElement("span");
      probe.style.backgroundColor = `var(${background})`;
      probe.style.borderTop = `1px solid var(${background})`;
      probe.style.color = `var(${text})`;
      document.body.append(probe);
      const style = getComputedStyle(probe);
      const paint = {
        background: style.backgroundColor,
        border: style.borderTopColor,
        color: style.color,
      };
      probe.remove();
      return paint;
    },
    { background, text },
  );
}

for (const theme of ["light", "dark"] as const) {
  test(`a primary button paints the primary color in the ${theme} scheme`, async ({ page }) => {
    // No color transition, so the hover reads its final color at once.
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mountComponents(page, { html: BUTTONS, components: ["button"], theme });
    const primary = await tokenPaint(page, "--sn-color-primary", "--sn-color-primary-contrast");

    expect(await paintOf(page, "primary")).toEqual(primary);
    expect(await paintOf(page, "submit")).toEqual(primary);
    expect(await paintOf(page, "link")).toEqual(primary);
    expect((await paintOf(page, "secondary")).background).not.toBe(primary.background);

    // A pressed primary toggle shows the active color, as a pressed submit
    // button does.
    const active = await tokenPaint(
      page,
      "--sn-color-primary-active",
      "--sn-color-primary-contrast",
    );
    expect(await paintOf(page, "pressed")).toEqual(active);

    await page.locator("#primary").hover();
    expect(await paintOf(page, "primary")).toEqual(
      await tokenPaint(page, "--sn-color-primary-hover", "--sn-color-primary-contrast"),
    );
  });
}
