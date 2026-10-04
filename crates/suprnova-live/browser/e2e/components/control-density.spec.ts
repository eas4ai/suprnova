import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { expect, test, type Page } from "@playwright/test";

import { mountComponents } from "./support.js";

// The form controls at the default density, measured in each engine. The
// ceilings are the suprnova.app form fields the library sits beside,
// measured in Chromium at a 1280 px viewport and a 16 px root: 14.72 px
// control text in a 41 px field (43 px for a date), 12.8 px labels 6 px
// above their control, 12.5 px hints and errors, a 94.84 px three-row
// textarea, and 42 px buttons with 15.2 px text.
const SITE = {
  controlText: 14.72,
  field: 41,
  dateField: 43,
  label: 12.8,
  labelGap: 6,
  hint: 12.5,
  textarea: 94.84,
  button: 42,
  buttonText: 15.2,
  // The sign-in and register forms put 14 px between one field and the
  // next, measured from the bottom of a control or its hint.
  fieldGap: 14,
} as const;

// The rows a textarea gets when the view passes none: the macro's default,
// read from the shipped view so the measurement follows it.
const textareaView = readFileSync(
  join(
    dirname(fileURLToPath(import.meta.url)),
    "..",
    "..",
    "..",
    "components",
    "textarea",
    "textarea.html",
  ),
  "utf8",
);
const defaultRows = /\{% macro textarea\([^)]*\brows=(\d+)/.exec(textareaView)?.[1];
if (defaultRows === undefined) throw new Error("textarea.html declares no default rows");

const field = (name: string, label: string, control: string, hint = "", error = ""): string =>
  `<div class="sn-field" data-sn-field="${name}"><label class="sn-label" for="${name}">${label}</label>${control}<p class="sn-hint" id="${name}-hint">${hint}</p><output class="sn-error" id="${name}-error">${error}</output></div>`;

// What the form-family macros render, one of each control.
const FORM = `<main style="padding: 1.5rem; max-inline-size: 40rem">
${field("full_name", "Name", `<input class="sn-input" id="full_name" name="full_name" type="text" placeholder="Ada Lovelace" aria-describedby="full_name-hint full_name-error" value="">`, "The server reads it on every change.", "Enter your name.")}
${field("q", "Search", `<input class="sn-input sn-search-input" id="q" name="q" type="search" placeholder="Search" autocomplete="off" aria-describedby="q-hint q-error" value="">`)}
${field("country_sel", "Country", `<select class="sn-select" id="country_sel" name="country_sel" aria-describedby="country_sel-hint country_sel-error"><option value="">Choose a country</option><option value="ca">Canada</option></select>`)}
${field("bio", "About you", `<textarea class="sn-textarea" id="bio" name="bio" rows="${defaultRows}" placeholder="A line or two about yourself." aria-describedby="bio-hint bio-error">\n</textarea>`)}
${field("bio3", "About you", `<textarea class="sn-textarea" id="bio3" name="bio3" rows="3" aria-describedby="bio3-hint bio3-error">\n</textarea>`)}
${field("qty", "Quantity", `<input class="sn-input sn-number-input" id="qty" name="qty" type="number" inputmode="decimal" min="0" max="10" step="1" aria-describedby="qty-hint qty-error" value="">`)}
${field("password", "Password", `<sn-password-reveal class="sn-password"><input class="sn-input sn-password-input" id="password" name="password" type="password" autocomplete="current-password" aria-describedby="password-hint password-error"><button class="sn-password-toggle" type="button" aria-controls="password" aria-pressed="false">Show</button></sn-password-reveal>`)}
<sn-combobox class="sn-combobox"><label class="sn-combobox-label" for="country">Country</label><input type="hidden" id="country-value" name="country" value="" data-sn-combobox-value><input class="sn-input sn-combobox-input" id="country" name="country_query" type="text" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="country-listbox" autocomplete="off" value=""><ul class="sn-combobox-listbox" id="country-listbox" role="listbox" aria-label="Country suggestions" hidden><li class="sn-combobox-option" id="country-option-1" role="option" aria-selected="false" data-sn-value="ca">Canada</li></ul></sn-combobox>
<sn-date-picker class="sn-date"><label class="sn-date-label" for="when">Date</label><input class="sn-input sn-date-input" id="when" name="when" type="date" min="2026-01-01" max="2028-12-31" value=""><details class="sn-date-strips"><summary class="sn-date-summary">Pick the parts</summary><fieldset class="sn-date-strip" data-sn-part="year"><legend class="sn-date-legend">Year</legend><label class="sn-date-option"><input class="sn-date-radio" type="radio" name="when-year" value="2026">2026</label></fieldset></details></sn-date-picker>
<div><label class="sn-label" for="nickname">Nickname</label><input class="sn-input" id="nickname" name="nickname" type="text" value=""></div>
<label class="sn-checkbox"><input class="sn-checkbox-input" id="agree" name="agree" type="checkbox"> <span>I agree to the terms</span></label>
<label class="sn-switch"><input class="sn-switch-input" id="newsletter" name="newsletter" type="checkbox" role="switch"> <span>Send me the newsletter</span></label>
<fieldset class="sn-radio-group" id="plan"><legend>Plan</legend><label class="sn-radio"><input class="sn-radio-input" name="plan" type="radio" value="starter"> <span>Starter</span></label></fieldset>
<p><button class="sn-button" type="button" data-sn-variant="primary">Save</button> <button class="sn-button" type="submit" data-sn-variant="primary">Submit</button> <button class="sn-button" type="button" data-sn-variant="secondary">Cancel</button></p>
<nav class="sn-pagination" id="pages" aria-label="Pages"><span class="sn-page" aria-disabled="true">Previous</span><a class="sn-page" href="#pages" aria-current="page">1</a><button class="sn-page" type="button">Next</button><span class="sn-page-position">Page 1 of 4</span></nav>
<ul class="sn-load-more-list" id="feed" aria-label="Feed"></ul><button class="sn-load-more" type="button">Load more</button>
<div class="sn-upload" data-sn-upload="attachment"><label class="sn-upload-label" for="attachment">Attachment</label><input class="sn-upload-input" id="attachment" type="file" accept="image/png"><div class="sn-upload-controls" role="group" aria-label="Attachment upload controls"><button class="sn-upload-control" type="button">Cancel upload</button></div></div>
<input class="sn-file-input" id="avatar" name="avatar" type="file">
</main>`;

const COMPONENTS = [
  "field",
  "label",
  "input",
  "search-input",
  "select",
  "textarea",
  "number-input",
  "password-input",
  "combobox",
  "date-picker",
  "checkbox",
  "switch",
  "radio-group",
  "button",
  "pagination",
  "load-more",
  "upload",
  "file-input",
] as const;

// The labelled controls, by the id of the control.
const LABELLED = [
  "full_name",
  "q",
  "country_sel",
  "bio",
  "bio3",
  "qty",
  "password",
  "country",
  "when",
  "nickname",
] as const;

interface Measured {
  readonly controls: Record<
    string,
    { font: number; height: number; label: number; labelGap: number }
  >;
  readonly hints: readonly number[];
  readonly buttons: readonly { text: string; font: number; height: number }[];
  readonly files: readonly { id: string; font: number }[];
  readonly choices: readonly { kind: string; font: number }[];
}

async function measure(page: Page): Promise<Measured> {
  return page.evaluate((ids) => {
    const px = (value: string): number => Number.parseFloat(value);
    const controls: Measured["controls"] = {};
    for (const id of ids) {
      const control = document.getElementById(id);
      const label = document.querySelector(`label[for="${id}"]`);
      if (control === null || label === null) throw new Error(`no control or label for ${id}`);
      const box = control.getBoundingClientRect();
      controls[id] = {
        font: px(getComputedStyle(control).fontSize),
        height: box.height,
        label: px(getComputedStyle(label).fontSize),
        labelGap: box.top - label.getBoundingClientRect().bottom,
      };
    }
    return {
      controls,
      hints: [...document.querySelectorAll(".sn-hint, .sn-error")]
        .filter((element) => element.textContent !== "")
        .map((element) => px(getComputedStyle(element).fontSize)),
      buttons: [...document.querySelectorAll("button, .sn-page")].map((button) => ({
        text: button.textContent,
        font: px(getComputedStyle(button).fontSize),
        height: button.getBoundingClientRect().height,
      })),
      files: [...document.querySelectorAll('input[type="file"]')].map((file) => ({
        id: file.id,
        font: px(getComputedStyle(file).fontSize),
      })),
      choices: [
        ...document.querySelectorAll(".sn-checkbox, .sn-switch, .sn-radio, .sn-date-option"),
      ].map((choice) => ({ kind: choice.className, font: px(getComputedStyle(choice).fontSize) })),
    };
  }, LABELLED);
}

test("the form controls are no larger than the suprnova.app form fields", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await mountComponents(page, { html: FORM, components: COMPONENTS });
  const measured = await measure(page);

  // Every measurement over its ceiling, so one run names them all.
  const over: string[] = [];
  const atMost = (what: string, value: number, ceiling: number): void => {
    if (value > ceiling + 0.01) over.push(`${what} ${String(value)} > ${String(ceiling)}`);
  };
  for (const [id, control] of Object.entries(measured.controls)) {
    atMost(`${id} text`, control.font, SITE.controlText);
    atMost(`${id} label`, control.label, SITE.label);
    atMost(`${id} label gap`, control.labelGap, SITE.labelGap);
    const ceiling =
      id === "bio" || id === "bio3" ? SITE.textarea : id === "when" ? SITE.dateField : SITE.field;
    atMost(`${id} height`, control.height, ceiling);
  }
  for (const hint of measured.hints) atMost("hint or error", hint, SITE.hint);
  for (const button of measured.buttons) {
    atMost(`${button.text} button text`, button.font, SITE.buttonText);
    atMost(`${button.text} button height`, button.height, SITE.button);
  }
  for (const file of measured.files) atMost(`${file.id} file text`, file.font, SITE.controlText);
  expect(over).toEqual([]);
  expect(measured.buttons).toHaveLength(9);
  expect(measured.files).toHaveLength(2);
  expect(measured.hints).toHaveLength(2);

  // A choice label keeps the 14 px small size: the smaller field label must
  // not shrink the text a person clicks to check a box. The site's own
  // choice labels are 14.08 px.
  expect(measured.choices).toHaveLength(4);
  for (const choice of measured.choices) expect(choice.font, choice.kind).toBe(14);
});

// A form as an application writes one: fields in plain flow, then the same
// controls in a fieldset, whose grid gap spaces its parts; the components
// that put their label in their own grid or row; and plain labels around a
// checkbox and a radio button, with no component class.
const RHYTHM = `<main style="padding: 1.5rem; max-inline-size: 40rem">
<form id="flow">
${field("flow_name", "Name", `<input class="sn-input" id="flow_name" name="flow_name" type="text">`, "", "Enter your name.")}
${field("flow_password", "Password", `<input class="sn-input" id="flow_password" name="flow_password" type="password">`, "At least 12 characters.")}
${field("flow_email", "Email", `<input class="sn-input" id="flow_email" name="flow_email" type="email">`)}
</form>
<form><fieldset class="sn-fieldset" id="grouped"><legend>Account</legend>
${field("set_name", "Name", `<input class="sn-input" id="set_name" name="set_name" type="text">`, "The server reads it on every change.")}
${field("set_email", "Email", `<input class="sn-input" id="set_email" name="set_email" type="email">`)}
<fieldset class="sn-radio-group" id="set_plan"><legend>Plan</legend><label class="sn-radio"><input class="sn-radio-input" name="set_plan" type="radio" value="starter"> <span>Starter</span></label></fieldset>
<label class="sn-checkbox"><input class="sn-checkbox-input" id="set_agree" name="set_agree" type="checkbox"> <span>I agree to the terms</span></label>
<label class="sn-switch"><input class="sn-switch-input" id="set_news" name="set_news" type="checkbox" role="switch"> <span>Send me the newsletter</span></label>
${field("set_city", "City", `<input class="sn-input" id="set_city" name="set_city" type="text">`)}
</fieldset></form>
<sn-input-otp class="sn-otp" data-sn-length="6"><label class="sn-otp-label" for="code">One-time code</label><input class="sn-input sn-otp-input" id="code" name="code" type="text" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" spellcheck="false" autocapitalize="off" required><span class="sn-otp-cells" aria-hidden="true"><span class="sn-otp-cell" data-sn-index="1"></span><span class="sn-otp-cell" data-sn-index="2"></span></span></sn-input-otp>
<div class="sn-upload" data-sn-upload="attachment"><label class="sn-upload-label" for="attachment">Attachment</label><input class="sn-upload-input" id="attachment" type="file" accept="image/png"><progress class="sn-upload-progress" max="100" aria-label="Attachment upload progress"></progress><p class="sn-upload-status"><span class="sn-upload-state" data-sn-state="idle">No file chosen.</span></p><div class="sn-upload-controls" role="group" aria-label="Attachment upload controls"><button class="sn-upload-control" type="button">Cancel upload</button></div></div>
<form class="sn-datatable-filter" role="search" aria-label="Filter invoices"><label class="sn-datatable-filter-label" for="invoices-filter">Filter invoices</label><input class="sn-datatable-filter-input" id="invoices-filter" name="filter" type="search" value=""><button class="sn-datatable-filter-button" type="submit">Apply</button></form>
<label id="bare-check"><input type="checkbox" name="remember"> Remember me</label>
<label id="bare-radio"><input type="radio" name="billing" value="monthly"> Monthly</label>
</main>`;

test("fields and their labels sit no farther apart than on the suprnova.app forms", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await mountComponents(page, {
    html: RHYTHM,
    components: [...COMPONENTS, "fieldset", "input-otp", "datatable"],
  });
  await expect(page.locator("sn-input-otp")).toHaveAttribute("data-sn-upgraded", "");
  const measured = await page.evaluate(() => {
    const box = (selector: string): DOMRect => {
      const element = document.querySelector(selector);
      if (element === null) throw new Error(`no ${selector}`);
      return element.getBoundingClientRect();
    };
    // The space from the bottom of one part (its hint or error included)
    // to the top of the next.
    const between = (parts: readonly Element[]): number[] =>
      parts
        .slice(1)
        .map((part, index) =>
          Math.round(
            part.getBoundingClientRect().top -
              (parts[index]?.getBoundingClientRect().bottom ?? Number.NaN),
          ),
        );
    const visibleChildren = (selector: string): Element[] =>
      [...document.querySelectorAll(`${selector} > :not(legend)`)].filter(
        (child) => child.getBoundingClientRect().height > 0,
      );
    const gap = (above: string, below: string): number =>
      Math.round(box(below).top - box(above).bottom);
    return {
      flow: between(visibleChildren("#flow")),
      fieldset: between(visibleChildren("#grouped")),
      labelGaps: {
        otp: gap(".sn-otp-label", "#code"),
        upload: gap(".sn-upload-label", "#attachment"),
        filter: gap(".sn-datatable-filter-label", "#invoices-filter"),
      },
      innerGaps: {
        otpCells: gap("#code", ".sn-otp-cells"),
        uploadStatus: gap("#attachment", ".sn-upload-status"),
        uploadControls: gap(".sn-upload-status", ".sn-upload-controls"),
        filterButton: gap("#invoices-filter", ".sn-datatable-filter-button"),
      },
      bareChoices: ["#bare-check", "#bare-radio"].map((selector) =>
        Number.parseFloat(
          getComputedStyle(document.querySelector(selector) ?? document.body).fontSize,
        ),
      ),
    };
  });

  expect(measured.flow).toHaveLength(2);
  expect(measured.fieldset).toHaveLength(5);
  const over = [
    ...measured.flow.map((space, index) => ["flow field", index, space] as const),
    ...measured.fieldset.map((space, index) => ["fieldset part", index, space] as const),
  ]
    .filter(([, , space]) => space > SITE.fieldGap)
    .map(([where, index, space]) => `${where} ${String(index)} to the next: ${String(space)}`);
  expect.soft(over).toEqual([]);

  // A label in its component's own grid or row sits as close to its control
  // as a field label does, and the parts below it keep their 8 px.
  expect.soft(measured.labelGaps).toEqual({ otp: 6, upload: 6, filter: 6 });
  expect.soft(measured.innerGaps).toEqual({
    otpCells: 8,
    uploadStatus: 8,
    uploadControls: 8,
    filterButton: 8,
  });

  // A plain label around a checkbox or radio button reads at the checkbox
  // component's 14 px, not the 12 px of a label above a field.
  expect.soft(measured.bareChoices).toEqual([14, 14]);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true });

  test("text fields keep 16 px text, so iOS Safari does not zoom on focus", async ({ page }) => {
    await mountComponents(page, { html: FORM, components: COMPONENTS });
    const coarse = await page.evaluate(() => matchMedia("(pointer: coarse)").matches);
    expect(coarse, "the context emulates a coarse pointer").toBe(true);
    const measured = await measure(page);
    for (const [id, control] of Object.entries(measured.controls)) {
      expect(control.font, id).toBeGreaterThanOrEqual(16);
    }
    for (const file of measured.files) expect(file.font, file.id).toBeGreaterThanOrEqual(16);
  });
});
