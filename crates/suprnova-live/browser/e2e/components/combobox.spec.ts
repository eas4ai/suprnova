import { expect, test, type Page } from "@playwright/test";

import { expectResponsive, mountComponents, within } from "./support.js";

// The shipped sn-combobox in each engine (FORM-008, FORM-011, FORM-012).
// The markup is what suprnova.combobox renders: a remote listbox carries
// data-sn-remote and the query the server rendered it for, and a local
// listbox carries neither.
function combobox(listbox: {
  remote: boolean;
  query?: string;
  options: readonly string[];
}): string {
  const remote = listbox.remote ? ` data-sn-remote data-sn-query="${listbox.query ?? ""}"` : "";
  const options = listbox.options
    .map(
      (text, index) =>
        `<li class="sn-combobox-option" id="country-option-${String(index + 1)}" role="option" aria-selected="false" data-sn-value="${text.toLowerCase()}">${text}</li>`,
    )
    .join("");
  return `<sn-combobox class="sn-combobox"><label class="sn-combobox-label" for="country">Country</label><input class="sn-input sn-combobox-input" id="country" name="country" type="text" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="country-listbox" autocomplete="off" value="${listbox.query ?? ""}"><ul class="sn-combobox-listbox" id="country-listbox" role="listbox" aria-label="Country suggestions"${remote} hidden>${options}</ul></sn-combobox><div id="elsewhere"></div>`;
}

const COUNTRIES = ["Canada", "Cameroon", "Chile", "Germany"] as const;

async function typeQuery(page: Page, text: string): Promise<void> {
  await within(page.locator("#country").focus(), 3_000, "focusing the input");
  await within(page.keyboard.type(text), 3_000, `typing ${JSON.stringify(text)}`);
}

// The server's answer to a query: new options and the query they answer, as
// a morph writes them.
async function answer(page: Page, query: string, options: readonly string[]): Promise<void> {
  await within(
    page.evaluate(
      ({ query, options }) => {
        const listbox = document.getElementById("country-listbox");
        if (listbox === null) throw new Error("listbox_missing");
        listbox.replaceChildren(
          ...options.map((text, index) => {
            const option = document.createElement("li");
            option.className = "sn-combobox-option";
            option.id = `country-option-${String(index + 1)}`;
            option.setAttribute("role", "option");
            option.setAttribute("aria-selected", "false");
            option.textContent = text;
            return option;
          }),
        );
        listbox.setAttribute("data-sn-query", query);
        listbox.hidden = true;
      },
      { query, options },
    ),
    3_000,
    "the server's answer",
  );
}

test("FORM-011: text that matches no option leaves the page responsive", async ({ page }) => {
  await mountComponents(page, {
    html: combobox({ remote: false, options: COUNTRIES }),
    components: ["combobox"],
  });
  await typeQuery(page, "zz");
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeHidden();
  await expect(page.locator("#country")).toHaveAttribute("aria-expanded", "false");

  await within(page.keyboard.press("Backspace"), 3_000, "deleting a character");
  await within(page.keyboard.press("Backspace"), 3_000, "deleting a character");
  await typeQuery(page, "ch");
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeVisible();
  await expect(page.locator("#country-listbox [role=option]:visible")).toHaveText(["Chile"]);
});

test("FORM-011: a remote answer with no options for the typed text leaves the page responsive", async ({
  page,
}) => {
  await mountComponents(page, {
    html: combobox({ remote: true, options: COUNTRIES }),
    components: ["combobox"],
  });
  await typeQuery(page, "zz");
  await answer(page, "zz", []);
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeHidden();
});

test("FORM-012: a remote listbox answering the input's text shows every option the server chose", async ({
  page,
}) => {
  await mountComponents(page, {
    html: combobox({ remote: true, options: COUNTRIES }),
    components: ["combobox"],
  });
  await typeQuery(page, "sao");
  await answer(page, "sao", ["São Tomé and Príncipe", "Brazil (São Paulo)"]);
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeVisible();
  await expect(page.locator("#country-listbox [role=option]:visible")).toHaveText([
    "São Tomé and Príncipe",
    "Brazil (São Paulo)",
  ]);
  await within(page.keyboard.press("ArrowDown"), 3_000, "moving to the first option");
  await within(page.keyboard.press("Enter"), 3_000, "selecting the option");
  await expect(page.locator("#country")).toHaveValue("São Tomé and Príncipe");
  await expect(page.locator("#country-listbox")).toBeHidden();
});

test("FORM-008: a listbox rendered for an older query stays hidden until the newer answer arrives", async ({
  page,
}) => {
  await mountComponents(page, {
    html: combobox({ remote: true, options: COUNTRIES }),
    components: ["combobox"],
  });
  await typeQuery(page, "ca");
  await answer(page, "ca", ["Canada", "Cameroon"]);
  await expect(page.locator("#country-listbox [role=option]:visible")).toHaveText([
    "Canada",
    "Cameroon",
  ]);

  await within(page.keyboard.type("n"), 3_000, "typing a newer query");
  await expect(page.locator("#country-listbox")).toBeHidden();
  // The answer to "ca" arrives again, after the user typed "can": it is stale.
  await answer(page, "ca", ["Canada", "Cameroon"]);
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeHidden();
  await expect(page.locator("#country")).toHaveAttribute("aria-expanded", "false");

  await answer(page, "can", ["Canada"]);
  await expect(page.locator("#country-listbox")).toBeVisible();
  await expect(page.locator("#country-listbox [role=option]:visible")).toHaveText(["Canada"]);
});

test("FORM-008: a morph that resets the listbox's hidden attribute keeps the popup open", async ({
  page,
}) => {
  await mountComponents(page, {
    html: combobox({ remote: false, options: COUNTRIES }),
    components: ["combobox"],
  });
  await typeQuery(page, "c");
  await expect(page.locator("#country-listbox")).toBeVisible();
  await within(
    page.evaluate(() => {
      const listbox = document.getElementById("country-listbox");
      if (listbox !== null) listbox.hidden = true;
    }),
    3_000,
    "the morph",
  );
  await expectResponsive(page);
  await expect(page.locator("#country-listbox")).toBeVisible();
});
