import { expect, test } from "@playwright/test";

import { mountComponents } from "./support.js";

// DATA-001 on the shipped files in each engine: the presentational
// primitives keep their shape inside the layouts a page puts them in.

test("DATA-001: the horizontal separator spans a grid track", async ({ page }) => {
  await mountComponents(page, {
    html: `<div style="display: grid; width: 400px"><p>Before</p><hr class="sn-separator" id="rule"><p>After</p></div>`,
    components: ["separator"],
  });
  const box = await page.locator("#rule").boundingBox();
  if (box === null) throw new Error("rule has no box");
  expect(box.width).toBeGreaterThan(300);
});

test("DATA-001: the scroll area scrolls content taller than its bound", async ({ page }) => {
  const items = Array.from(
    { length: 40 },
    (_, index) => `<li>Entry ${String(index + 1)}</li>`,
  ).join("");
  await mountComponents(page, {
    html: `<div class="sn-scroll-area" id="log" role="region" aria-label="Log" tabindex="0" data-sn-axis="block"><ol>${items}</ol></div>`,
    components: ["scroll-area"],
  });
  const log = page.locator("#log");
  const sizes = await log.evaluate((node) => ({
    scroll: node.scrollHeight,
    client: node.clientHeight,
  }));
  expect(sizes.scroll).toBeGreaterThan(sizes.client);
  expect(sizes.client).toBeLessThan(400);
  await log.evaluate((node) => {
    node.scrollTo(0, 200);
  });
  expect(await log.evaluate((node) => node.scrollTop)).toBe(200);
});

test("DATA-002: every member of an avatar group sits on one row", async ({ page }) => {
  await mountComponents(page, {
    html: `<ul class="sn-avatar-group" role="list" aria-label="Reviewers"><li class="sn-avatar-member"><span class="sn-avatar" role="img" aria-label="Ada Lovelace" data-sn-size="md">AL</span></li><li class="sn-avatar-member"><span class="sn-avatar" role="img" aria-label="Grace Hopper" data-sn-size="md">GH</span></li><li class="sn-avatar-member"><span class="sn-avatar" role="img" aria-label="Linus Torvalds" data-sn-size="md">LT</span></li></ul>`,
    components: ["avatar"],
  });
  const tops = await page
    .locator(".sn-avatar-member")
    .evaluateAll((items) => items.map((item) => Math.round(item.getBoundingClientRect().top)));
  expect(tops).toHaveLength(3);
  expect(new Set(tops).size).toBe(1);
});
