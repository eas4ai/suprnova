import { chromium, type Browser, type Page } from "@playwright/test";
import { buildSync } from "esbuild";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, beforeEach, expect, test } from "vitest";

interface DateField extends HTMLElement {
  value: string;
}

const script = buildSync({
  entryPoints: [
    fileURLToPath(
      new URL("../../components/segmented-date-picker/segmented-date-picker.js", import.meta.url),
    ),
  ],
  bundle: true,
  write: false,
  format: "iife",
}).outputFiles[0]?.text;

let browser: Browser;
let page: Page;

beforeAll(async () => {
  browser = await chromium.launch();
});

beforeEach(async () => {
  page = await browser.newPage();
  await page.setContent(
    '<sn-segmented-date id="f" value="2026-09-23" scroller="false"></sn-segmented-date>',
  );
  if (script === undefined) throw new Error("component_script_missing");
  await page.addScriptTag({ content: script });
  await page.locator('[data-segment="month"]').focus();
});

afterEach(async () => {
  await page.close();
});

afterAll(async () => {
  await browser.close();
});

test.each([false, true])("insertText applies once with cancelable=%s", async (cancelable) => {
  const result = await page.locator('[data-segment="month"]').evaluate((month, cancelable) => {
    const field = document.querySelector<DateField>("#f");
    if (!field) throw new Error("field_missing");
    const before = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable,
      inputType: "insertText",
      data: "1",
    });
    month.dispatchEvent(before);
    const beforeText = month.textContent;
    month.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "insertText", data: "1" }),
    );
    return {
      prevented: before.defaultPrevented,
      beforeText,
      text: month.textContent,
      value: field.value,
      focused: document.activeElement === month,
    };
  }, cancelable);
  expect(result).toEqual({
    prevented: cancelable,
    beforeText: cancelable ? "1_" : "09",
    text: "1_",
    value: "2026-09-23",
    focused: true,
  });
  await page.keyboard.type("2");
  expect(await page.locator("#f").evaluate((field: DateField) => field.value)).toBe("2026-12-23");
});

test.each([false, true])("deletion applies once with cancelable=%s", async (cancelable) => {
  await page.keyboard.type("1");
  const result = await page.locator('[data-segment="month"]').evaluate((month, cancelable) => {
    const field = document.querySelector<DateField>("#f");
    if (!field) throw new Error("field_missing");
    const before = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable,
      inputType: "deleteContentBackward",
    });
    month.dispatchEvent(before);
    const beforeText = month.textContent;
    month.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward" }),
    );
    return { beforeText, text: month.textContent, value: field.value };
  }, cancelable);
  expect(result).toEqual({
    beforeText: cancelable ? "09" : "1_",
    text: "09",
    value: "2026-09-23",
  });
});

test("an ineffective preventDefault leaves the edit to input", async () => {
  const result = await page.locator('[data-segment="month"]').evaluate((month) => {
    const before = new InputEvent("beforeinput", {
      bubbles: true,
      cancelable: true,
      inputType: "insertText",
      data: "1",
    });
    before.preventDefault = () => undefined;
    month.dispatchEvent(before);
    const beforeText = month.textContent;
    month.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "insertText", data: "1" }),
    );
    return { prevented: before.defaultPrevented, beforeText, text: month.textContent };
  });
  expect(result).toEqual({ prevented: false, beforeText: "09", text: "1_" });
});

test("a canceled beforeinput does not suppress a later native-only edit", async () => {
  await page.keyboard.type("1");
  await page.locator('[data-segment="month"]').evaluate((month) => {
    month.dispatchEvent(
      new InputEvent("input", { bubbles: true, inputType: "insertText", data: "2" }),
    );
  });
  expect(await page.locator("#f").evaluate((field: DateField) => field.value)).toBe("2026-12-23");
});
