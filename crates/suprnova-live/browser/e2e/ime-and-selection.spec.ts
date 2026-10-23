import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("text and contenteditable selection survive an active IME morph", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("continuity");
  await page.evaluate(() => {
    const input = document.querySelector("#continuity-selection");
    const editable = document.querySelector("#continuity-editable");
    if (!(input instanceof HTMLInputElement) || !(editable instanceof HTMLElement)) {
      throw new Error("selection fixtures missing");
    }
    input.value = "browser-selection";
    input.setSelectionRange(2, 9, "backward");
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true, data: "選" }));
    input.dispatchEvent(new CompositionEvent("compositionupdate", { bubbles: true, data: "選択" }));
    const range = document.createRange();
    const text = editable.firstChild;
    if (text === null) throw new Error("editable text missing");
    range.setStart(text, 1);
    range.setEnd(text, 8);
    const selection = document.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    Reflect.set(window, "__continuityInput", input);
    const action = document.querySelector("#continuity-action");
    if (!(action instanceof HTMLElement)) throw new Error("action missing");
    action.click();
  });

  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "8");
  expect(
    await page.evaluate(() => {
      const input = document.querySelector("#continuity-selection");
      const selection = document.getSelection();
      return {
        editable: selection?.toString(),
        end: input instanceof HTMLInputElement ? input.selectionEnd : null,
        same: Reflect.get(window, "__continuityInput") === input,
        start: input instanceof HTMLInputElement ? input.selectionStart : null,
        value: input instanceof HTMLInputElement ? input.value : null,
      };
    }),
  ).toEqual({ editable: "ditable", end: 9, same: true, start: 2, value: "browser-selection" });
  await page.locator("#continuity-selection").dispatchEvent("compositionend", { data: "選択" });
});
