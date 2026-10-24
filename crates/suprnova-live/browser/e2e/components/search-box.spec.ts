import { expect, test } from "@playwright/test";
import { expectResponsive, mountComponents } from "./support.js";

function searchBox(query = "", sequence = 0): string {
  return `<sn-search-box class="sn-search-box"><label for="query">Search</label><input id="query" type="search" role="combobox" aria-autocomplete="list" aria-controls="query-listbox" aria-expanded="false" value="${query}"><input type="hidden" value="${String(sequence)}" data-sn-search-sequence><input type="hidden" data-sn-search-selection><input type="hidden" data-sn-search-position><button type="button" data-sn-search-select hidden>Select</button><ul id="query-listbox" role="listbox" aria-label="Search results" data-sn-query="${query}" data-sn-sequence="${String(sequence)}" hidden><li id="result-one" role="option" aria-selected="false" data-sn-key="one" data-sn-position="0"><mark>Journal</mark> &lt;script&gt;</li><li id="result-two" role="option" aria-selected="false" data-sn-key="two" data-sn-position="1">Notebook</li></ul></sn-search-box>`;
}

test("keyboard navigation retains input focus and selects through the island", async ({ page }) => {
  await mountComponents(page, { html: searchBox(), components: ["search-box"] });
  const input = page.getByRole("combobox");
  await input.focus();
  await input.press("ArrowDown");
  await expect(input).toHaveAttribute("aria-expanded", "true");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-one");
  await expect(input).toBeFocused();
  await input.press("ArrowDown");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-two");
  await input.press("ArrowUp");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-one");
  await input.press("End");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-two");
  await input.press("Home");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-one");
  await page.evaluate(() => {
    document.querySelector("[data-sn-search-select]")?.addEventListener("click", () => {
      document.body.setAttribute("data-selected", "accepted-by-island");
    });
  });
  await input.press("Enter");
  await expect(page.locator("[data-sn-search-selection]")).toHaveValue("one");
  await expect(page.locator("[data-sn-search-position]")).toHaveValue("0");
  await expect(page.locator("body")).toHaveAttribute("data-selected", "accepted-by-island");
  await expect(input).toHaveAttribute("aria-expanded", "false");
});

test("a late answer cannot replace the latest query, even when text repeats", async ({ page }) => {
  await mountComponents(page, { html: searchBox("journal"), components: ["search-box"] });
  const input = page.getByRole("combobox");
  await input.fill("notebook");
  await input.fill("journal");
  await expect(input).toHaveAttribute("aria-expanded", "false");
  await page.locator("[role=listbox]").evaluate((list) => {
    list.setAttribute("data-sn-query", "journal");
    list.setAttribute("data-sn-sequence", "1");
  });
  await expect(input).toHaveAttribute("aria-expanded", "false");
  await page.locator("[role=listbox]").evaluate((list) => {
    list.setAttribute("data-sn-sequence", "2");
  });
  await expect(input).toHaveAttribute("aria-expanded", "true");
  await page.locator("[role=listbox]").evaluate((list) => {
    list.setAttribute("data-sn-sequence", "1");
  });
  await expect(input).toHaveAttribute("aria-expanded", "false");
});

test("server fuzzy results retain escaped text and highlight markup", async ({ page }) => {
  await mountComponents(page, { html: searchBox("jourmal"), components: ["search-box"] });
  await page.getByRole("combobox").focus();
  await expect(page.getByRole("option")).toHaveCount(2);
  await expect(page.locator("[role=option] mark")).toHaveText("Journal");
  await expect(page.locator("[role=option]").first()).toContainText("<script>");
  await expect(page.locator("[role=option] script")).toHaveCount(0);
  await page.getByRole("combobox").press("Escape");
  await expect(page.getByRole("combobox")).toHaveAttribute("aria-expanded", "false");
});

test("an empty answer stays responsive without observer feedback", async ({ page }) => {
  await mountComponents(page, { html: searchBox(), components: ["search-box"] });
  await page.getByRole("combobox").fill("unmatched");
  await page.locator("[role=listbox]").evaluate((list) => {
    list.replaceChildren();
    list.setAttribute("data-sn-query", "unmatched");
    list.setAttribute("data-sn-sequence", "1");
  });
  await expectResponsive(page);
  await expect(page.getByRole("combobox")).toHaveAttribute("aria-expanded", "false");
});

test("a disconnect releases observers and event listeners", async ({ page }) => {
  await mountComponents(page, { html: searchBox(), components: ["search-box"], instrument: true });
  await page.locator("sn-search-box").evaluate((element) => {
    element.remove();
  });
  const bindings: unknown = await page.evaluate("globalThis.__snBindings()");
  expect(bindings).toEqual({ listeners: 0, observers: 0 });
});

test("a document without the component script defines no search-box tag", async ({ page }) => {
  await mountComponents(page, { html: searchBox(), components: ["search-box"], scripts: false });
  expect(await page.evaluate(() => customElements.get("sn-search-box") === undefined)).toBe(true);
});

test("initial ArrowUp chooses the last result and pointer selection reaches the island", async ({
  page,
}) => {
  await mountComponents(page, { html: searchBox(), components: ["search-box"] });
  const input = page.getByRole("combobox");
  await input.focus();
  await input.press("ArrowUp");
  await expect(input).toHaveAttribute("aria-activedescendant", "result-two");
  await input.press("Enter");
  await expect(page.locator("[data-sn-search-selection]")).toHaveValue("two");
  await input.press("ArrowDown");
  await expect(input).toHaveAttribute("aria-expanded", "true");
  await page.getByRole("option").first().click();
  await expect(page.locator("[data-sn-search-selection]")).toHaveValue("one");
});
