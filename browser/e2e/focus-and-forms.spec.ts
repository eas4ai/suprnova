import { expect, test } from "@playwright/test";
import axe from "axe-core";

import { RuntimePage } from "./support/runtime-page.js";

test("keyed morphs preserve focus, dirty controls, files, and scoped scroll while explicit correction wins", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("continuity");
  await page.locator("#continuity-text").fill("newer browser text");
  await page.locator("#continuity-correction").fill("browser correction candidate");
  await page.locator("#continuity-check").check();
  await page.locator("#continuity-radio-b").check();
  await page.locator("#continuity-select").selectOption("b");
  await page.locator("#continuity-multiple").selectOption(["b", "c"]);
  await page.locator("#continuity-file").setInputFiles({
    buffer: Buffer.from("owned file"),
    mimeType: "text/plain",
    name: "owned.txt",
  });
  await page.evaluate(() => {
    Reflect.set(window, "__continuityFile", document.querySelector("#continuity-file"));
    const scroll = document.querySelector("#continuity-scroll");
    if (!(scroll instanceof HTMLElement)) throw new Error("scroll scope missing");
    scroll.scrollTop = 150;
  });
  await page.locator("#continuity-action").focus();
  await page.keyboard.press("Tab");
  await expect(page.locator("#continuity-focused")).toBeFocused();
  expect(
    await page
      .locator("#continuity-focused")
      .evaluate((element) => element.matches(":focus-visible")),
  ).toBe(true);
  await page.evaluate(() => {
    const action = document.querySelector("#continuity-action");
    if (!(action instanceof HTMLElement)) throw new Error("action missing");
    action.click();
  });

  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#continuity-focused")).toBeFocused();
  expect(
    await page
      .locator("#continuity-focused")
      .evaluate((element) => element.matches(":focus-visible")),
  ).toBe(true);
  await expect(page.locator("#continuity-focused")).toMatchAriaSnapshot(`- textbox: focus me`);
  await expect(page.locator("#continuity-text")).toHaveValue("newer browser text");
  await expect(page.locator("#continuity-correction")).toHaveValue("corrected-8");
  await expect(page.locator("#continuity-check")).toBeChecked();
  await expect(page.locator("#continuity-radio-b")).toBeChecked();
  await expect(page.locator("#continuity-select")).toHaveValue("b");
  await expect(page.locator("#continuity-multiple")).toHaveValues(["b", "c"]);
  expect(
    await page.locator("#continuity-file").evaluate((element) => ({
      count: element instanceof HTMLInputElement ? element.files?.length : -1,
      same: Reflect.get(window, "__continuityFile") === element,
    })),
  ).toEqual({ count: 1, same: true });
  expect(await page.locator("#continuity-scroll").evaluate((element) => element.scrollTop)).toBe(
    150,
  );

  await page.evaluate(() => {
    const focused = document.querySelector("#continuity-focused");
    if (!(focused instanceof HTMLElement)) throw new Error("focus target missing");
    focused.focus();
    const action = document.querySelector("#continuity-action");
    if (!(action instanceof HTMLElement)) throw new Error("action missing");
    action.click();
  });
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "9");
  await expect(page.locator("#continuity-focused")).toHaveCount(0);
  await expect(page.locator("#continuity-fallback")).toBeFocused();

  await page.evaluate(() => {
    const focused = document.querySelector("#continuity-default-focused");
    const action = document.querySelector("#continuity-action");
    if (!(focused instanceof HTMLElement) || !(action instanceof HTMLElement)) {
      throw new Error("default focus fixtures missing");
    }
    focused.focus();
    action.click();
  });
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "10");
  await expect(page.locator("#continuity-default-focused")).toHaveCount(0);
  await expect(page.locator("#continuity-fallback")).toHaveCount(0);
  await expect(runtime.island()).toBeFocused();

  await page.addScriptTag({ content: axe.source });
  expect(
    await runtime.island().evaluate(async (root) => {
      const axeRuntime = (
        window as unknown as {
          axe: { run(target: Element): Promise<{ violations: readonly unknown[] }> };
        }
      ).axe;
      return (await axeRuntime.run(root)).violations;
    }),
  ).toEqual([]);
});
