import { expect, test, type Page } from "@playwright/test";

import { mountComponents } from "./support.js";

type Segment = "year" | "month" | "day";
interface DateField extends HTMLElement {
  value: string;
  valueAsParts: Record<Segment, number | null>;
  validity: ValidityState;
  validationMessage: string;
  checkValidity(): boolean;
  formStateRestoreCallback(value: string): void;
}
interface Sample {
  t: number;
  off: number | null;
  labels: string[];
}
declare global {
  interface Window {
    dateEvents: [string, string, string | null][];
    dateSamples: Sample[];
  }
}

const segment = (page: Page, name: Segment) => page.locator(`[data-segment="${name}"]`);
const value = (page: Page) => page.locator("#f").evaluate((field: DateField) => field.value);
const centre = (page: Page) =>
  page.locator('.sn-segmented-item[aria-current="true"]').textContent();
const scroller = (page: Page) => page.locator(".sn-segmented-scroller");

// The macro's light-DOM editor, with the unnamed native Live bridge. The
// dogfood tests separately compile and render the macro through Askama.
async function mount(page: Page, theme: "light" | "dark", attrs: Record<string, string> = {}) {
  const parts = ["year", "month", "day"]
    .map(
      (name) =>
        `<span class="sn-segmented-segment" data-segment="${name}" role="spinbutton" aria-label="${name}" inputmode="numeric" contenteditable="true" tabindex="0"></span>`,
    )
    .join("");
  const attributes = Object.entries({ order: "ymd", ...attrs })
    .map(([key, text]) => `${key}="${text}"`)
    .join(" ");
  await mountComponents(page, {
    theme,
    html: `<main style="padding:40px"><form id="form"><label id="start-label" for="f">Start date</label><sn-segmented-date id="f" class="sn-segmented-date" name="start" role="group" aria-labelledby="start-label" ${attributes}><input type="text" hidden id="canonical" data-sn-value live:model="start" value="${attrs["value"] ?? ""}"><div data-sn-editor live:key="start-editor" live:ignore.children><div class="sn-segmented-field">${parts}</div><span class="sn-segmented-sr-only" role="status" aria-live="polite"></span></div></sn-segmented-date><input id="after" aria-label="After"></form></main>`,
    components: ["segmented-date-picker"],
  });
  await page.evaluate(() => {
    window.dateEvents = [];
    const field = document.querySelector<DateField>("#f");
    if (!field) throw new Error("field missing");
    for (const kind of ["input", "change", "focussegment", "blursegment", "open", "close"]) {
      field.addEventListener(kind, (event) => {
        const detail: unknown = event instanceof CustomEvent ? event.detail : null;
        const name =
          detail && typeof detail === "object" && "segment" in detail
            ? String(detail.segment)
            : null;
        window.dateEvents.push([kind, field.value, name]);
      });
    }
  });
}
async function entries(page: Page) {
  return page
    .locator("#form")
    .evaluate((form: HTMLFormElement) => [...new FormData(form).entries()]);
}
async function paste(page: Page, name: Segment, text: string) {
  // Real clipboard input also covers Firefox, which ignores clipboardData
  // on synthetic ClipboardEvents.
  await page.evaluate((text) => {
    const input = document.createElement("textarea");
    input.id = "clipboard-source";
    input.value = text;
    document.body.append(input);
    input.focus();
    input.select();
  }, text);
  await page.keyboard.press("ControlOrMeta+C");
  await page.locator("#clipboard-source").evaluate((node) => {
    node.remove();
  });
  await segment(page, name).focus();
  await page.keyboard.press("ControlOrMeta+V");
}
async function sample(
  page: Page,
  label: string,
  milliseconds: number,
  action: () => Promise<void>,
) {
  await page.evaluate(
    ({ label, milliseconds }) => {
      window.dateSamples = [];
      const start = performance.now();
      const frame = () => {
        const viewport = document.querySelector<HTMLElement>(".sn-segmented-viewport");
        const track = document.querySelector<HTMLElement>(".sn-segmented-scroller");
        if (!viewport || !track) throw new Error("scroller missing");
        const width = Number.parseFloat(track.style.getPropertyValue("--sn-segmented-item-width"));
        let off: number | null = null;
        const labels: string[] = [];
        for (const item of document.querySelectorAll<HTMLElement>(".sn-segmented-item")) {
          if (item.hidden || item.style.visibility === "hidden") continue;
          const x = Number.parseFloat(
            /translate3d\(([-\d.e]+)px/.exec(item.style.transform)?.[1] ?? "NaN",
          );
          const distance = x + width / 2 - viewport.clientWidth / 2;
          if (Number(item.style.opacity) > 0.05) labels.push(item.textContent);
          if (item.textContent === label && (off === null || Math.abs(distance) < Math.abs(off)))
            off = distance;
        }
        window.dateSamples.push({ t: performance.now() - start, off, labels });
        if (performance.now() - start < milliseconds) requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
    },
    { label, milliseconds },
  );
  await action();
  await expect
    .poll(() => page.evaluate(() => window.dateSamples[window.dateSamples.length - 1]?.t ?? 0))
    .toBeGreaterThanOrEqual(milliseconds);
  return page.evaluate(() => window.dateSamples);
}

for (const theme of ["light", "dark"] as const) {
  test.describe(`FORM-013 ${theme}`, () => {
    for (const [order, names] of [
      ["ymd", ["year", "month", "day"]],
      ["dmy", ["day", "month", "year"]],
      ["mdy", ["month", "day", "year"]],
    ] as const) {
      test(`AC1-2: ${order} display, labels and traversal`, async ({ page }) => {
        await mount(page, theme, { order, value: "2026-09-23" });
        expect(
          await page
            .locator("[data-segment]")
            .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset["segment"])),
        ).toEqual(names);
        await page.locator("label").click();
        for (const name of names) {
          await expect(segment(page, name)).toBeFocused();
          await page.keyboard.press("Tab");
        }
        await expect(page.locator("#after")).toBeFocused();
        await page.keyboard.press("Shift+Tab");
        await expect(segment(page, names[2])).toBeFocused();
        expect(await page.locator(".sn-segmented-separator").allTextContents()).toEqual(["/", "/"]);
      });
    }
    test("AC3: direct digits, separators, shorthand and auto-advance", async ({ page }) => {
      for (const [order, first, text, date] of [
        ["mdy", "month", "09232026", "2026-09-23"],
        ["mdy", "month", "9/23/2026", "2026-09-23"],
        ["dmy", "day", "5/1/2026", "2026-01-05"],
        ["ymd", "year", "2026-9-23", "2026-09-23"],
      ] as const) {
        await mount(page, theme, { order });
        await segment(page, first).focus();
        await page.keyboard.type(text);
        expect(await value(page)).toBe(date);
      }
      await mount(page, theme, { "auto-advance": "false" });
      await segment(page, "year").focus();
      await page.keyboard.type("2026");
      await expect(segment(page, "year")).toBeFocused();
      await page.keyboard.press("Tab");
      await page.keyboard.type("09");
      await expect(segment(page, "month")).toBeFocused();
    });
    test("AC4: reject invalid digits; incomplete buffers revert", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      await segment(page, "month").focus();
      await page.keyboard.type("19");
      await expect(segment(page, "month")).toHaveText("1_");
      expect(await value(page)).toBe("2026-09-23");
      await expect(page.locator("[role=status]")).toContainText("19 is not a valid month");
      await page.keyboard.press("Escape");
      await segment(page, "year").focus();
      await page.keyboard.type("20");
      await page.keyboard.press("Tab");
      expect(await value(page)).toBe("2026-09-23");
      await segment(page, "day").focus();
      await page.keyboard.type("31");
      await expect(segment(page, "day")).toHaveText("3_");
      await page.keyboard.press("Tab");
      expect(await value(page)).toBe("2026-09-03");
    });
    test("AC5-6: contextual scroller follows focus and Escape", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      await expect(scroller(page)).toBeHidden();
      for (const [name, label] of [
        ["month", "09"],
        ["year", "2026"],
        ["day", "23"],
      ] as const) {
        await segment(page, name).focus();
        await expect(scroller(page)).toBeVisible();
        await expect.poll(() => centre(page)).toBe(label);
      }
      await page.keyboard.press("Escape");
      await expect(scroller(page)).toBeHidden();
      await expect(segment(page, "day")).toBeFocused();
      await page.keyboard.press("ArrowUp");
      await expect(scroller(page)).toBeVisible();
      await page.locator("#after").focus();
      await expect(scroller(page)).toBeHidden();
      expect(
        await page.evaluate(() =>
          window.dateEvents
            .filter(([kind]) => kind === "open" || kind === "close")
            .map(([kind]) => kind),
        ),
      ).toEqual(["open", "close", "open", "close"]);
    });
    test("AC7: typing and pending previews follow one committed state", async ({ page }) => {
      await mount(page, theme, { value: "2024-02-29", "auto-advance": "false" });
      await segment(page, "year").focus();
      await page.keyboard.type("1");
      await expect.poll(() => centre(page)).toBe("1999");
      await expect(scroller(page)).toHaveAttribute("data-sn-preview", "");
      await page.keyboard.type("98");
      await expect.poll(() => centre(page)).toBe("1989");
      expect(await value(page)).toBe("2024-02-29");
      expect(
        await page.evaluate(() => window.dateEvents.filter(([kind]) => kind === "input")),
      ).toEqual([]);
      await page.keyboard.press("Backspace");
      await expect.poll(() => centre(page)).toBe("1999");
      await page.keyboard.press("Escape");
      await expect.poll(() => centre(page)).toBe("2024");
      await page.keyboard.type("1987");
      expect(await value(page)).toBe("1987-02-28");
      await segment(page, "month").focus();
      await page.keyboard.type("1");
      await expect.poll(() => centre(page)).toBe("01");
      await page.keyboard.type("2");
      expect(await value(page)).toBe("1987-12-29");
      await expect.poll(() => centre(page)).toBe("12");
    });
    test("AC8: buttons, hold repeat and item taps update the segment", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      await segment(page, "month").focus();
      await page.locator(".sn-segmented-next").click();
      expect(await value(page)).toBe("2026-10-23");
      await page.locator(".sn-segmented-prev").click();
      await page.locator(".sn-segmented-prev").click();
      await expect.poll(() => centre(page)).toBe("08");
      // A visible centre label can arrive before its settle finishes. Wait
      // for the selected item to reach the exact centre before tapping a
      // different item in the virtual pool.
      await expect
        .poll(() =>
          page.locator('.sn-segmented-item[aria-current="true"]').evaluate((node) => {
            const viewport = node.closest(".sn-segmented-viewport");
            if (!viewport) return Infinity;
            const a = node.getBoundingClientRect(),
              b = viewport.getBoundingClientRect();
            return Math.abs(a.x + a.width / 2 - b.x - b.width / 2);
          }),
        )
        .toBeLessThan(0.01);
      await page.locator(".sn-segmented-item").getByText("10", { exact: true }).click();
      expect(await value(page)).toBe("2026-10-23");
      await expect(segment(page, "month")).toBeFocused();
      await page.locator(".sn-segmented-next").dispatchEvent("pointerdown", { button: 0 });
      await expect.poll(() => value(page)).not.toBe("2026-11-23");
      await page.locator(".sn-segmented-next").dispatchEvent("pointerup");
    });
    test("AC9-10: wrapping and bounded years, Home and End", async ({ page }) => {
      await mount(page, theme, { value: "2026-12-31" });
      await segment(page, "month").focus();
      await page.keyboard.press("ArrowUp");
      expect(await value(page)).toBe("2026-01-31");
      await page.keyboard.press("ArrowDown");
      await segment(page, "day").focus();
      await page.keyboard.press("ArrowUp");
      expect(await value(page)).toBe("2026-12-01");
      await page.keyboard.press("ArrowDown");
      expect(await value(page)).toBe("2026-12-31");
      await mount(page, theme, { value: "2030-06-15", min: "2020-03-10", max: "2030-08-20" });
      await segment(page, "year").focus();
      await page.keyboard.press("ArrowUp");
      expect(await value(page)).toBe("2030-06-15");
      await expect(page.locator(".sn-segmented-next")).toBeDisabled();
      await page.keyboard.press("Home");
      await segment(page, "month").focus();
      await page.keyboard.press("Home");
      await segment(page, "day").focus();
      await page.keyboard.press("Home");
      expect(await value(page)).toBe("2020-03-10");
      await segment(page, "year").focus();
      await page.keyboard.type("2031");
      await expect(segment(page, "year")).toHaveText("203_");
      await page.keyboard.press("Escape");
      await page.keyboard.press("End");
      expect(await value(page)).toBe("2030-03-10");
    });
    test("AC11-12: dependent day clamps and restores preferred intent", async ({ page }) => {
      await mount(page, theme, { value: "2026-01-31" });
      await segment(page, "month").focus();
      await page.keyboard.type("4");
      expect(await value(page)).toBe("2026-04-30");
      await expect(page.locator("[role=status]")).toHaveText("Day adjusted to 30");
      await expect(segment(page, "day")).toHaveAttribute("data-sn-adjusted", "");
      await expect(segment(page, "day")).toBeFocused();
      const colors = await segment(page, "day").evaluate((node) => [
        getComputedStyle(node).backgroundColor,
        getComputedStyle(node).color,
      ]);
      expect(colors[0]).not.toBe(colors[1]);
      await mount(page, theme, { value: "2024-02-29" });
      await segment(page, "year").focus();
      await page.keyboard.type("202");
      expect(await value(page)).toBe("2024-02-29");
      await page.keyboard.type("5");
      expect(await value(page)).toBe("2025-02-28");
      await segment(page, "year").focus();
      await page.keyboard.type("2028");
      expect(await value(page)).toBe("2028-02-29");
      await page.locator("#after").focus();
      await segment(page, "year").focus();
      await page.keyboard.type("2100");
      expect(await value(page)).toBe("2100-02-28");
      await page.locator("#after").focus();
      await segment(page, "year").focus();
      await page.keyboard.type("2000");
      expect(await value(page)).toBe("2000-02-28");
    });
    test("AC13-14: drag snaps, preserves focus, and virtualises the track", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-10", motion: "none" });
      await segment(page, "day").focus();
      const box = await page.locator(".sn-segmented-viewport").boundingBox();
      if (!box) throw new Error("viewport missing");
      const x = box.x + box.width / 2,
        y = box.y + box.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x - 96, y, { steps: 20 });
      await page.mouse.up();
      expect(await value(page)).toBe("2026-09-12");
      await expect.poll(() => centre(page)).toBe("12");
      await expect(segment(page, "day")).toBeFocused();
      await segment(page, "year").focus();
      expect(await page.locator(".sn-segmented-item").count()).toBeLessThan(20);
      const centered = await page
        .locator('.sn-segmented-item[aria-current="true"]')
        .evaluate((node) => {
          const viewport = node.closest(".sn-segmented-viewport");
          if (!viewport) throw new Error("viewport missing");
          const a = node.getBoundingClientRect(),
            b = viewport.getBoundingClientRect();
          return Math.abs(a.x + a.width / 2 - b.x - b.width / 2);
        });
      expect(centered).toBeLessThan(0.1);
    });
    test("AC13: fling momentum and vertical gesture isolation", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-10" });
      await segment(page, "day").focus();
      const box = await page.locator(".sn-segmented-viewport").boundingBox();
      if (!box) throw new Error("viewport missing");
      const x = box.x + box.width / 2,
        y = box.y + box.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x - 10, y);
      for (let i = 1; i <= 5; i++) {
        await page.evaluate(() => new Promise(requestAnimationFrame));
        await page.mouse.move(x - 10 - i * 24, y);
      }
      await page.mouse.up();
      expect(Number((await value(page)).slice(-2))).toBeGreaterThanOrEqual(13);
      await mount(page, theme, { value: "2026-09-10" });
      await segment(page, "day").focus();
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x - 9, y + 45, { steps: 9 });
      await page.mouse.up();
      expect(await value(page)).toBe("2026-09-10");
    });
    test("AC13: touch pointers, wheel notches and trackpad direction", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-10", motion: "none" });
      await segment(page, "day").focus();
      const viewport = page.locator(".sn-segmented-viewport");
      await viewport.evaluate((node) => {
        const box = node.getBoundingClientRect();
        const x = box.x + box.width / 2,
          y = box.y + box.height / 2;
        // Synthetic touch pointers exercise every engine's gesture logic.
        const capture = node.setPointerCapture.bind(node);
        node.setPointerCapture = () => undefined;
        try {
          const send = (type: string, x: number) =>
            node.dispatchEvent(
              new PointerEvent(type, {
                bubbles: true,
                cancelable: true,
                pointerId: 4,
                pointerType: "touch",
                button: 0,
                clientX: x,
                clientY: y,
              }),
            );
          send("pointerdown", x);
          send("pointermove", x + 10);
          send("pointermove", x + 106);
          send("pointerup", x + 106);
        } finally {
          node.setPointerCapture = capture;
        }
      });
      expect(await value(page)).toBe("2026-09-08");
      await scroller(page).dispatchEvent("wheel", { deltaY: 100, deltaMode: 0 });
      expect(await value(page)).toBe("2026-09-09");
      for (let i = 0; i < 12; i++)
        await scroller(page).dispatchEvent("wheel", { deltaX: 8, deltaY: 0, deltaMode: 0 });
      await expect.poll(() => value(page)).toBe("2026-09-11");
      await scroller(page).dispatchEvent("wheel", { deltaY: 3, deltaMode: 0 });
      expect(await value(page)).toBe("2026-09-11");
      await expect(viewport).toHaveCSS("touch-action", "pan-y");
    });
    test("AC15: one small overshoot and one settle", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      await segment(page, "month").focus();
      const samples = await sample(page, "10", 600, () => page.keyboard.press("ArrowUp"));
      const offsets = samples.flatMap((item) => (item.off === null ? [] : [item.off]));
      const minimum = Math.min(...offsets);
      expect(minimum).toBeGreaterThanOrEqual(-4.5);
      expect(minimum).toBeLessThanOrEqual(-1.5);
      const after = offsets.slice(offsets.indexOf(minimum));
      expect(after.every((off, i) => i === 0 || off >= (after[i - 1] ?? off) - 0.01)).toBe(true);
      expect(offsets[offsets.length - 1]).toBeCloseTo(0, 2);
    });
    test("AC16: typed large jumps enter near the destination", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23", order: "dmy" });
      await segment(page, "year").focus();
      const samples = await sample(page, "1978", 1100, async () => {
        await page.keyboard.type("1");
        await expect.poll(() => centre(page)).toBe("1999");
        await page.keyboard.type("978");
      });
      const seen = samples.flatMap((item) => item.labels).map(Number);
      expect(seen.some((year) => year >= 2005 && year <= 2020)).toBe(false);
      expect(await value(page)).toBe("1978-09-23");
      await expect.poll(() => centre(page)).toBe("1978");
    });
    test("AC17: reduced motion and motion=none snap without spring", async ({ page }) => {
      for (const reduce of [true, false]) {
        await page.emulateMedia({ reducedMotion: reduce ? "reduce" : "no-preference" });
        await mount(page, theme, { value: "2026-09-23", ...(reduce ? {} : { motion: "none" }) });
        await segment(page, "month").focus();
        const samples = await sample(page, "10", 300, () => page.keyboard.press("ArrowUp"));
        expect(
          samples.every(
            (item) =>
              item.off === null || Math.abs(item.off) < 0.01 || Math.abs(item.off - 48) < 0.01,
          ),
        ).toBe(true);
        expect(
          await page.locator("#f").evaluate((node) => node.getAnimations({ subtree: true }).length),
        ).toBe(0);
      }
    });
    test("AC18: typed and scrolled values use one canonical form and model value", async ({
      page,
    }) => {
      await mount(page, theme, { order: "dmy" });
      await segment(page, "day").focus();
      await page.keyboard.type("23092026");
      await expect(page.locator("[data-sn-value]")).toBeHidden();
      expect(await entries(page)).toEqual([["start", "2026-09-23"]]);
      await expect(page.locator("[data-sn-value]")).toHaveValue("2026-09-23");
      await segment(page, "day").focus();
      await page.locator(".sn-segmented-next").click();
      expect(await entries(page)).toEqual([["start", "2026-09-24"]]);
      await expect(page.locator("[data-sn-value]")).toHaveValue("2026-09-24");
      await segment(page, "day").focus();
      await page.keyboard.type("23");
      expect(await entries(page)).toEqual([["start", "2026-09-23"]]);
    });
    test("AC18: required, partial, bounds, reset, restore and dirty semantics", async ({
      page,
    }) => {
      await mount(page, theme, { required: "" });
      expect(
        await page.locator("#f").evaluate((field: DateField) => field.validity.valueMissing),
      ).toBe(true);
      await segment(page, "year").focus();
      await page.keyboard.type("2026");
      expect(
        await page
          .locator("#f")
          .evaluate((field: DateField) => field.validity.badInput && !field.checkValidity()),
      ).toBe(true);
      expect(await entries(page)).toEqual([]);
      await page.keyboard.type("0923");
      expect(await page.locator("#f").evaluate((field: DateField) => field.checkValidity())).toBe(
        true,
      );
      await page.locator("#f").evaluate((field: DateField) => {
        field.setAttribute("value", "2028-01-01");
      });
      expect(await value(page)).toBe("2026-09-23");
      await page.locator("#form").evaluate((form: HTMLFormElement) => {
        form.reset();
      });
      expect(await value(page)).toBe("2028-01-01");
      await page.locator("#f").evaluate((field: DateField) => {
        field.formStateRestoreCallback("2027-03-04");
      });
      expect(await value(page)).toBe("2027-03-04");
      for (const [date, flag] of [
        ["1899-12-31", "rangeUnderflow"],
        ["2101-01-01", "rangeOverflow"],
      ] as const) {
        expect(
          await page.locator("#f").evaluate(
            (field: DateField, { date, flag }) => {
              field.value = date;
              return field.validity[flag] && !field.checkValidity();
            },
            { date, flag },
          ),
        ).toBe(true);
        expect(await entries(page)).toEqual([]);
      }
    });
    test("AC19: named spinbuttons, numeric keyboard and hidden scroller", async ({
      page,
      browserName,
    }) => {
      await mount(page, theme, { value: "2026-09-23", order: "mdy" });
      await expect(page.getByRole("group", { name: "Start date" })).toBeVisible();
      for (const [name, text, min, max] of [
        ["month", "September, 9 of 12", "1", "12"],
        ["day", "23 of 30", "1", "30"],
        ["year", "2026", "1900", "2100"],
      ] as const) {
        const control = segment(page, name);
        await expect(control).toHaveAttribute(
          "aria-label",
          name.charAt(0).toUpperCase() + name.slice(1),
        );
        await expect(control).toHaveAttribute("aria-valuetext", text);
        await expect(control).toHaveAttribute("aria-valuemin", min);
        await expect(control).toHaveAttribute("aria-valuemax", max);
        await expect(control).toHaveAttribute("inputmode", "numeric");
      }
      await segment(page, "month").focus();
      await expect(scroller(page)).toHaveAttribute("aria-hidden", "true");
      expect(
        await page
          .locator(".sn-segmented-nav")
          .evaluateAll((nodes) => nodes.every((node) => (node as HTMLElement).tabIndex === -1)),
      ).toBe(true);
      expect(await page.getByRole("spinbutton").count()).toBe(3);
      if (browserName === "chromium") {
        // Preserve the source's native accessibility-tree assertion as
        // well as the portable role and attribute checks on every engine.
        interface AxNode {
          ignored?: boolean;
          role?: { value?: string };
          name?: { value?: string };
          value?: { value?: number };
          properties?: { name: string; value: { value?: unknown } }[];
        }
        const session = await page.context().newCDPSession(page);
        const tree = (await session.send("Accessibility.getFullAXTree")) as { nodes: AxNode[] };
        const spins = tree.nodes.filter(
          (node) => !node.ignored && node.role?.value === "spinbutton",
        );
        expect(spins.map((node) => node.name?.value)).toEqual(["Month", "Day", "Year"]);
        // Chromium 151's CDP serializer reads kValue for valuetext, but
        // the renderer stores aria-valuetext in kAriaValueText. The untouched
        // source and a plain ARIA spinbutton both reproduce the empty CDP
        // property. Keep strict value-text assertions above on every engine,
        // and prove native AX names, values, ranges and hidden content here.
        // https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/accessibility/inspector_type_builder_helper.cc
        // https://chromium.googlesource.com/chromium/src/+/main/third_party/blink/renderer/modules/accessibility/ax_object.cc
        expect(spins.map((node) => node.value?.value)).toEqual([9, 23, 2026]);
        expect(
          spins.map((node) =>
            ["valuemin", "valuemax"].map(
              (name) => node.properties?.find((property) => property.name === name)?.value.value,
            ),
          ),
        ).toEqual([
          [1, 12],
          [1, 30],
          [1900, 2100],
        ]);

        expect(
          tree.nodes.some(
            (node) =>
              !node.ignored &&
              node.role?.value === "StaticText" &&
              ["08", "10"].includes(node.name?.value ?? ""),
          ),
        ).toBe(false);
        await session.detach();
      }
      expect(await page.locator("#f").evaluate((node) => node.shadowRoot)).toBeNull();
    });
    test("AC20: fully usable with the scroller disabled", async ({ page }) => {
      await mount(page, theme, { scroller: "false" });
      await segment(page, "year").focus();
      await page.keyboard.type("2026-9-23");
      await page.keyboard.press("ArrowUp");
      expect(await value(page)).toBe("2026-09-24");
      await expect(scroller(page)).toBeHidden();
    });
    test("events, backspace, paste, disabled and readonly", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      await segment(page, "month").focus();
      await page.keyboard.press("ArrowUp");
      await page.keyboard.press("ArrowUp");
      await page.keyboard.press("Enter");
      await page.keyboard.press("ArrowDown");
      await page.locator("#after").focus();
      expect(
        await page.evaluate(() =>
          window.dateEvents
            .filter(([kind]) => kind === "input" || kind === "change")
            .map(([kind]) => kind),
        ),
      ).toEqual(["input", "input", "change", "input", "change"]);
      await segment(page, "month").focus();
      await page.keyboard.type("1");
      await page.keyboard.press("Backspace");
      await expect(segment(page, "month")).toHaveText("10");
      await page.keyboard.press("Backspace");
      expect(await value(page)).toBe("");
      await page.keyboard.press("Backspace");
      await expect(segment(page, "year")).toBeFocused();
      await paste(page, "day", "2031-12-05");
      expect(await value(page)).toBe("2031-12-05");
      await mount(page, theme, { value: "2026-09-23", readonly: "" });
      await segment(page, "month").focus();
      await page.keyboard.press("ArrowUp");
      await page.keyboard.type("12");
      expect(await value(page)).toBe("2026-09-23");
      await expect(scroller(page)).toBeHidden();
      await mount(page, theme, { value: "2026-09-23", disabled: "" });
      expect(await entries(page)).toEqual([]);
      await expect(segment(page, "year")).toHaveAttribute("aria-disabled", "true");
    });
    test("local today default, month labels, flip and 44px targets", async ({ page }) => {
      await mount(page, theme, { "default-today": "" });
      const today = await page.evaluate(() => {
        const d = new Date();
        return `${String(d.getFullYear())}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
      });
      expect(await value(page)).toBe(today);
      await mount(page, theme, { "default-today": "", max: "2020-06-30" });
      expect(await value(page)).toBe("2020-06-30");
      await mount(page, theme, { value: "2026-09-23", "month-display": "short", locale: "en-US" });
      await segment(page, "month").focus();
      await expect.poll(() => centre(page)).toBe("Sep");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("month-display", "long");
      });
      await expect.poll(() => centre(page)).toBe("September");
      for (const button of await page.locator(".sn-segmented-nav").all()) {
        const box = await button.boundingBox();
        expect(box?.width).toBeGreaterThanOrEqual(43.99);
        expect(box?.height).toBeGreaterThanOrEqual(43.99);
      }
      await page.locator("#f").evaluate((field: HTMLElement) => {
        field.style.position = "fixed";
        field.style.bottom = "10px";
        field.style.left = "10px";
      });
      await page.setViewportSize({ width: 800, height: 600 });
      await expect(scroller(page)).toHaveAttribute("data-side", "top");
      const box = await scroller(page).boundingBox(),
        field = await page.locator(".sn-segmented-field").boundingBox();
      if (!box || !field) throw new Error("positioned control missing");
      expect(box.y + box.height).toBeLessThanOrEqual(field.y);
      expect(box.x).toBeGreaterThanOrEqual(8);
    });
    test("bounds changes reconcile pending buffers and partial leap days", async ({ page }) => {
      await mount(page, theme, { value: "2024-02-29" });
      await segment(page, "year").focus();
      await page.keyboard.type("20");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("min", "2010-01-01");
        field.setAttribute("max", "2030-12-31");
      });
      await expect(segment(page, "year")).toHaveText("20__");
      await page.keyboard.type("28");
      expect(await value(page)).toBe("2028-02-29");
      await segment(page, "year").focus();
      await page.keyboard.type("20");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("min", "1950-01-01");
        field.setAttribute("max", "1999-12-31");
      });
      await expect(segment(page, "year")).toHaveText("");
      await expect(page.locator("[role=status]")).toHaveText("Incomplete year discarded");
      await mount(page, theme);
      await segment(page, "month").focus();
      await page.keyboard.type("0229");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("min", "2023-01-01");
        field.setAttribute("max", "2025-12-31");
      });
      await expect(segment(page, "day")).toHaveText("29");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("min", "2025-01-01");
        field.setAttribute("max", "2027-12-31");
      });
      await expect(segment(page, "day")).toHaveText("");
    });
    test("popover and ARIA fallbacks keep the pointer control usable", async ({ page }) => {
      await page.evaluate(() => {
        Reflect.deleteProperty(HTMLElement.prototype, "showPopover");
        Reflect.deleteProperty(HTMLElement.prototype, "hidePopover");
        Reflect.deleteProperty(ElementInternals.prototype, "role");
        Reflect.deleteProperty(ElementInternals.prototype, "ariaDisabled");
      });
      await mount(page, theme, { value: "2026-09-23" });
      await expect(scroller(page)).not.toHaveAttribute("popover");
      await segment(page, "month").focus();
      await expect(scroller(page)).toBeVisible();
      await page.locator(".sn-segmented-next").click();
      expect(await value(page)).toBe("2026-10-23");
      await page.locator("#f").evaluate((field) => {
        field.setAttribute("disabled", "");
      });
      await expect(page.locator("#f")).toHaveAttribute("aria-disabled", "true");
      await expect(scroller(page)).toBeHidden();
    });
    test("tokens, density and removing the base skin keep the same value", async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await mount(page, theme, { value: "2026-09-23" });
      const field = page.locator(".sn-segmented-field");
      const normal = await field.evaluate((node) => node.getBoundingClientRect().height);
      await page.evaluate(() => {
        document.documentElement.style.setProperty("--sn-density-scale", "0.75");
      });
      expect(await field.evaluate((node) => node.getBoundingClientRect().height)).toBeLessThan(
        normal,
      );
      // The first stylesheet is the shared base; the component keeps its
      // structural CSS and its native semantics when that skin is absent.
      await page
        .locator("style")
        .first()
        .evaluate((node) => {
          node.remove();
        });
      await segment(page, "day").focus();
      await page.keyboard.press("ArrowUp");
      expect(await entries(page)).toEqual([["start", "2026-09-24"]]);
      expect(errors).toEqual([]);
    });
    test("direct elements support detached values and parser defaults", async ({ page }) => {
      await mount(page, theme);
      await page.locator("#form").evaluate((form) => {
        const field = document.createElement("sn-segmented-date") as DateField;
        field.id = "f";
        field.setAttribute("name", "start");
        field.setAttribute("order", "ymd");
        field.value = "2026-09-23";
        form.replaceChildren(field);
      });
      await expect.poll(() => value(page)).toBe("2026-09-23");
      await segment(page, "day").focus();
      await page.keyboard.press("ArrowUp");
      expect(await entries(page)).toEqual([["start", "2026-09-24"]]);
      await page.locator("#form").evaluate((form) => {
        form.innerHTML =
          '<sn-segmented-date id="f" name="start" default-today max="2020-06-30"></sn-segmented-date>';
      });
      await expect.poll(() => value(page)).toBe("2020-06-30");
      expect(await entries(page)).toEqual([["start", "2020-06-30"]]);
    });
    test("reconnect keeps one editor, one scroller and one event per change", async ({ page }) => {
      await mount(page, theme, { value: "2026-09-23" });
      for (let i = 0; i < 3; i++)
        await page.locator("#f").evaluate((node) => {
          const parent = node.parentElement;
          node.remove();
          parent?.append(node);
        });
      await segment(page, "day").focus();
      await page.locator(".sn-segmented-next").click();
      expect(await value(page)).toBe("2026-09-24");
      expect(await scroller(page).count()).toBe(1);
      expect(
        await page.evaluate(() => window.dateEvents.filter(([kind]) => kind === "input").length),
      ).toBe(1);
      // An authoritative reset can keep the original value attribute
      // unchanged while replacing the dirty native value property.
      await page.locator("[data-sn-value]").evaluate((input: HTMLInputElement) => {
        input.setAttribute("data-suprnova-live-authoritative", "1");
        input.value = input.defaultValue;
      });
      await expect.poll(() => value(page)).toBe("2026-09-23");
      expect(await entries(page)).toEqual([["start", "2026-09-23"]]);
      await page.locator("[data-sn-value]").evaluate((input: HTMLInputElement) => {
        input.setAttribute("value", "2027-02-03");
        input.value = "2027-02-03";
      });
      await expect.poll(() => value(page)).toBe("2027-02-03");
    });
  });
}
