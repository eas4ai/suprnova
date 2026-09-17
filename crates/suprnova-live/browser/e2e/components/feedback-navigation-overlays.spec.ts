import { expect, test, type Page } from "@playwright/test";

import { mountComponents } from "./support.js";

// FDB-007, NAV-007, OVL-007, and UI-024 on the shipped files in each engine.

async function center(page: Page, selector: string): Promise<{ x: number; y: number }> {
  const box = await page.locator(selector).boundingBox();
  if (box === null) throw new Error(`${selector} has no box`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

test.describe("FDB-007: the toast region holds a toast's timer", () => {
  const REGION = `<main id="page"><p id="away">Elsewhere</p><button id="outside" type="button">Outside</button><sn-toast-region class="sn-toast-region" data-sn-limit="3"><div class="sn-toast-list" id="toasts" role="status" aria-live="polite" aria-label="Notifications"><div class="sn-toast" id="saved" data-sn-variant="info" data-sn-duration="1000"><span class="sn-toast-text" id="saved-text">Saved</span><button class="sn-toast-dismiss" id="saved-dismiss" type="button" data-sn-toast-dismiss aria-label="Dismiss">&#215;</button></div></div></sn-toast-region></main>`;

  test.beforeEach(async ({ page }) => {
    await page.clock.install();
    await mountComponents(page, { html: REGION, components: ["toast"] });
    await expect(page.locator("#saved")).toBeVisible();
  });

  test("while the pointer moves from the text to the padding", async ({ page }) => {
    const text = await center(page, "#saved-text");
    await page.mouse.move(text.x, text.y);
    const toast = await page.locator("#saved").boundingBox();
    const span = await page.locator("#saved-text").boundingBox();
    if (toast === null || span === null) throw new Error("toast has no box");
    // A point inside the toast and outside its text: the padding beside it.
    const padding = { x: toast.x + 1, y: toast.y + 1 };
    expect(padding.x < span.x || padding.y < span.y).toBe(true);
    await page.mouse.move(padding.x, padding.y, { steps: 4 });
    await page.clock.runFor(3_000);
    await expect(page.locator("#saved")).toBeVisible();

    await page.mouse.move(1, 1);
    await page.clock.runFor(1_100);
    await expect(page.locator("#saved")).toBeHidden();
  });

  test("while focus is on its dismiss button and the pointer leaves", async ({ page }) => {
    const button = await center(page, "#saved-dismiss");
    await page.mouse.move(button.x, button.y);
    await page.locator("#saved-dismiss").focus();
    await page.mouse.move(1, 1);
    await page.clock.runFor(3_000);
    await expect(page.locator("#saved")).toBeVisible();

    await page.locator("#outside").focus();
    await page.clock.runFor(1_100);
    await expect(page.locator("#saved")).toBeHidden();
  });
});

test("NAV-007: a nested local tabs instance selects only its own tabs and panels", async ({
  page,
}) => {
  const tabs = (id: string, names: readonly string[], inner: (name: string) => string): string =>
    `<sn-tabs class="sn-tabs" id="${id}" data-sn-mode="local" data-sn-label="${id}"><div class="sn-tablist" role="tablist" aria-label="${id}">${names
      .map(
        (name, index) =>
          `<button class="sn-tab" type="button" role="tab" id="${id}-${name}" aria-controls="${id}-${name}-panel" aria-selected="${String(index === 0)}"${index === 0 ? "" : ' tabindex="-1"'}>${name}</button>`,
      )
      .join("")}</div>${names
      .map(
        (name, index) =>
          `<div class="sn-tab-panel" role="tabpanel" id="${id}-${name}-panel" aria-labelledby="${id}-${name}" tabindex="0"${index === 0 ? "" : " hidden"}>${inner(name)}</div>`,
      )
      .join("")}</sn-tabs>`;
  const html = tabs("outer", ["billing", "usage"], (name) =>
    name === "billing" ? tabs("inner", ["cards", "invoices"], (inner) => inner) : name,
  );
  await mountComponents(page, { html, components: ["tabs"] });

  await page.locator("#inner-invoices").click();
  await expect(page.locator("#inner-invoices-panel")).toBeVisible();
  await expect(page.locator("#inner-cards-panel")).toBeHidden();
  await expect(page.locator("#outer-billing-panel")).toBeVisible();
  await expect(page.locator("#outer-billing")).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#outer-usage-panel")).toBeHidden();

  await page.keyboard.press("ArrowRight");
  await expect(page.locator("#inner-cards")).toBeFocused();
  await expect(page.locator("#inner-cards-panel")).toBeVisible();
  await expect(page.locator("#outer-billing-panel")).toBeVisible();
  await expect(page.locator("#outer-billing")).toHaveAttribute("aria-selected", "true");

  await page.locator("#outer-usage").click();
  await expect(page.locator("#outer-usage-panel")).toBeVisible();
  await expect(page.locator("#outer-billing-panel")).toBeHidden();
  await expect(page.locator("#inner-cards")).toHaveAttribute("aria-selected", "true");
});

test("OVL-007: the tooltip bubble stays visible while the pointer moves onto it", async ({
  page,
}) => {
  await mountComponents(page, {
    html: `<main style="padding: 4rem"><sn-tooltip class="sn-tooltip"><button class="sn-button" id="trigger" type="button" aria-describedby="tip">Save</button><span class="sn-tooltip-bubble" id="tip" role="tooltip">Saves the notes to the server</span></sn-tooltip></main>`,
    components: ["tooltip"],
  });
  // The fade is not under test; without it, visibility follows :hover at once.
  await page.addStyleTag({ content: ".sn-tooltip-bubble { transition: none !important; }" });
  const trigger = await center(page, "#trigger");
  await page.mouse.move(trigger.x, trigger.y);
  await expect(page.locator("#tip")).toBeVisible();
  const bubble = await center(page, "#tip");
  await page.mouse.move(bubble.x, bubble.y, { steps: 12 });
  await expect(page.locator("#tip")).toBeVisible();
  expect(
    await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.id, bubble),
    "the bubble takes the pointer",
  ).toBe("tip");

  await page.mouse.move(1, 1);
  await expect(page.locator("#tip")).toBeHidden();
});

interface IndicatorMeasure {
  readonly surface: number;
  readonly farthest: number;
  readonly text: number;
  readonly darkest: number;
  readonly lightest: number;
}

/** Luminances of the select's indicator area beside its surface and text color. */
async function measureIndicator(page: Page): Promise<IndicatorMeasure> {
  const shot = await page.locator("#country").screenshot();
  return page.evaluate(async (png) => {
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d");
    if (context === null) throw new Error("no_canvas");
    context.drawImage(image, 0, 0);
    const luminance = (red: number, green: number, blue: number): number =>
      0.2126 * red + 0.7152 * green + 0.0722 * blue;
    // The indicator sits in the right-hand 40 CSS pixels, clear of the text
    // and inside the border.
    const scale =
      image.width / (document.getElementById("country")?.getBoundingClientRect().width ?? 1);
    const data = context.getImageData(
      Math.round(image.width - 40 * scale),
      Math.round(4 * scale),
      Math.round(32 * scale),
      Math.round(image.height - 8 * scale),
    ).data;
    const values: number[] = [];
    for (let index = 0; index < data.length; index += 4) {
      values.push(luminance(data[index] ?? 0, data[index + 1] ?? 0, data[index + 2] ?? 0));
    }
    values.sort((left, right) => left - right);
    const surface = values[Math.floor(values.length / 2)] ?? 0;
    const select = document.getElementById("country");
    if (select === null) throw new Error("no_select");
    const color = getComputedStyle(select).color;
    const probe = document.createElement("canvas").getContext("2d");
    if (probe === null) throw new Error("no_probe");
    probe.fillStyle = color;
    probe.fillRect(0, 0, 1, 1);
    const [red, green, blue] = probe.getImageData(0, 0, 1, 1).data;
    return {
      surface,
      farthest: Math.max(
        Math.abs((values[0] ?? 0) - surface),
        Math.abs((values[values.length - 1] ?? 0) - surface),
      ),
      text: luminance(red ?? 0, green ?? 0, blue ?? 0),
      darkest: values[0] ?? 0,
      lightest: values[values.length - 1] ?? 0,
    };
  }, shot.toString("base64"));
}

for (const theme of ["light", "dark"] as const) {
  test(`UI-024: the select indicator draws in the text color in the ${theme} scheme`, async ({
    page,
  }) => {
    await mountComponents(page, {
      html: `<main style="padding: 1.5rem"><select id="country" style="inline-size: 15rem"><option>Choose a country</option></select></main>`,
      components: [],
      theme,
    });
    const measured = await measureIndicator(page);
    // The indicator stands out from the surface, and on the side of the text
    // color: lighter than the surface in the dark scheme, darker in the light.
    expect(measured.farthest, JSON.stringify(measured)).toBeGreaterThan(40);
    if (measured.text > measured.surface) {
      expect(measured.lightest - measured.surface, JSON.stringify(measured)).toBeGreaterThan(40);
    } else {
      expect(measured.surface - measured.darkest, JSON.stringify(measured)).toBeGreaterThan(40);
    }
  });
}

test("UI-024: the select keeps an indicator where forced colors drop background images", async ({
  page,
}) => {
  await page.emulateMedia({ forcedColors: "active" });
  await mountComponents(page, {
    html: `<main style="padding: 1.5rem"><select id="country" style="inline-size: 15rem"><option>Choose a country</option></select></main>`,
    components: [],
  });
  const forced = await page.evaluate(() => matchMedia("(forced-colors: active)").matches);
  test.skip(!forced, "this engine does not emulate forced colors");
  const appearance = await page
    .locator("#country")
    .evaluate((select) => getComputedStyle(select).appearance);
  expect(appearance).toBe("auto");
  const measured = await measureIndicator(page);
  expect(measured.farthest, JSON.stringify(measured)).toBeGreaterThan(40);
});
