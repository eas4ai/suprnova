import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("nested ownership and shadow boundaries stay local", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("directiveOwnership");
  await runtime.expectStatus("connected", 0);
  await runtime.expectStatus("connected", 1);

  const results = await page.evaluate(() => {
    const click = (target: EventTarget | null) =>
      target?.dispatchEvent(
        new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
      );
    const open = document.querySelector("#open-host")?.shadowRoot?.querySelector("button") ?? null;
    const closed = (
      window as typeof window & { __suprnovaClosedButton?: HTMLButtonElement }
    ).__suprnovaClosedButton;
    return {
      childOwned: click(document.querySelector("#child-owned")),
      childPlain: click(document.querySelector("#child-plain")),
      closed: click(closed ?? null),
      open: click(open),
    };
  });

  expect(results).toEqual({ childOwned: false, childPlain: true, closed: true, open: false });
});
