import { expect, test } from "@playwright/test";

import { mountComponents } from "./support.js";

// UI-020: a library custom element keeps one set of listeners and observers
// however often a morph moves or reconnects it, and releases them when it
// leaves the document. Idiomorph moves a persistent-id node with moveBefore
// where the engine has it and with insertBefore where it does not; a morph
// also strips the attributes the server never rendered, such as an upgrade
// marker, from the host. The markup is what each component's view renders.
interface Fixture {
  readonly component: string;
  readonly element: string;
  readonly html: string;
}

const FIXTURES: readonly Fixture[] = [
  {
    component: "combobox",
    element: "sn-combobox",
    html: `<sn-combobox class="sn-combobox"><label class="sn-combobox-label" for="country">Country</label><input class="sn-input sn-combobox-input" id="country" name="country" type="text" role="combobox" aria-autocomplete="list" aria-expanded="false" aria-controls="country-listbox" autocomplete="off" value="" list="country-datalist"><datalist id="country-datalist"><option value="Canada"></option></datalist><ul class="sn-combobox-listbox" id="country-listbox" role="listbox" aria-label="Country suggestions" data-sn-remote data-sn-query="" hidden><li class="sn-combobox-option" id="country-option-1" role="option" aria-selected="false" data-sn-value="ca">Canada</li></ul></sn-combobox>`,
  },
  {
    component: "input-otp",
    element: "sn-input-otp",
    html: `<sn-input-otp class="sn-otp" data-sn-length="6"><label class="sn-otp-label" for="code">One-time code</label><input class="sn-input sn-otp-input" id="code" name="code" type="text" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" spellcheck="false" autocapitalize="off" required><span class="sn-otp-cells" aria-hidden="true"><span class="sn-otp-cell" data-sn-index="0"></span><span class="sn-otp-cell" data-sn-index="1"></span></span></sn-input-otp>`,
  },
  {
    component: "date-picker",
    element: "sn-date-picker",
    html: `<sn-date-picker class="sn-date"><label class="sn-date-label" for="when">Renewal date</label><input class="sn-input sn-date-input" id="when" name="when" type="date" min="2026-01-01" max="2028-12-31"><details class="sn-date-strips"><summary class="sn-date-summary">Pick the parts</summary><fieldset class="sn-date-strip" data-sn-part="year"><legend class="sn-date-legend">Year</legend><label class="sn-date-option"><input class="sn-date-radio" type="radio" name="when-year" value="2026">2026</label></fieldset><fieldset class="sn-date-strip" data-sn-part="month"><legend class="sn-date-legend">Month</legend><label class="sn-date-option"><input class="sn-date-radio" type="radio" name="when-month" value="1">January</label></fieldset><fieldset class="sn-date-strip" data-sn-part="day"><legend class="sn-date-legend">Day</legend><label class="sn-date-option"><input class="sn-date-radio" type="radio" name="when-day" value="1">1</label></fieldset></details></sn-date-picker>`,
  },
  {
    component: "password-input",
    element: "sn-password-reveal",
    html: `<sn-password-reveal class="sn-password"><input class="sn-input sn-password-input" id="password" name="password" type="password" autocomplete="current-password"><button class="sn-password-toggle" type="button" aria-controls="password" aria-pressed="false">Show</button></sn-password-reveal>`,
  },
  {
    component: "dialog",
    element: "sn-dialog",
    html: `<button class="sn-button" type="button" data-sn-dialog-open="dialog-one">Open</button><sn-dialog class="sn-dialog-host" tabindex="-1"><dialog class="sn-dialog" id="dialog-one" aria-labelledby="dialog-one-title" closedby="any"><h2 id="dialog-one-title">Title</h2><div><button class="sn-button" type="button" data-sn-dialog-close="dialog-one">Close</button></div></dialog></sn-dialog>`,
  },
  {
    component: "sheet",
    element: "sn-sheet",
    html: `<button class="sn-button" type="button" data-sn-sheet-open="sheet-one">Open</button><sn-sheet class="sn-sheet-host" tabindex="-1"><dialog class="sn-sheet" id="sheet-one" aria-labelledby="sheet-one-title" closedby="any"><h2 id="sheet-one-title">Title</h2><div><button class="sn-button" type="button" data-sn-sheet-close="sheet-one">Close</button></div></dialog></sn-sheet>`,
  },
  {
    component: "drawer",
    element: "sn-drawer",
    html: `<button class="sn-button" type="button" data-sn-drawer-open="drawer-one">Open</button><sn-drawer class="sn-drawer-host" tabindex="-1"><dialog class="sn-drawer" id="drawer-one" aria-labelledby="drawer-one-title" closedby="any"><h2 id="drawer-one-title">Title</h2><div><button class="sn-button" type="button" data-sn-drawer-close="drawer-one">Close</button></div></dialog></sn-drawer>`,
  },
  {
    component: "tabs",
    element: "sn-tabs",
    html: `<sn-tabs class="sn-tabs" id="plans" data-sn-mode="local" data-sn-label="Plans"><div class="sn-tablist" role="tablist" aria-label="Plans"><button class="sn-tab" type="button" role="tab" id="monthly" aria-controls="monthly-panel" aria-selected="true">Monthly</button><button class="sn-tab" type="button" role="tab" id="yearly" aria-controls="yearly-panel" aria-selected="false" tabindex="-1">Yearly</button></div><div class="sn-tab-panel" role="tabpanel" id="monthly-panel" aria-labelledby="monthly" tabindex="0">Monthly</div><div class="sn-tab-panel" role="tabpanel" id="yearly-panel" aria-labelledby="yearly" tabindex="0" hidden>Yearly</div></sn-tabs>`,
  },
  {
    component: "toast",
    element: "sn-toast-region",
    html: `<sn-toast-region class="sn-toast-region" data-sn-limit="3"><div class="sn-toast-list" id="toasts" role="status" aria-live="polite" aria-label="Notifications"><div class="sn-toast" data-sn-variant="info" data-sn-duration="60000"><span class="sn-toast-text">Saved</span><button class="sn-toast-dismiss" type="button" data-sn-toast-dismiss aria-label="Dismiss">&#215;</button></div></div></sn-toast-region>`,
  },
];

for (const fixture of FIXTURES) {
  test(`UI-020: ${fixture.element} keeps one set of listeners and observers across morph moves`, async ({
    page,
  }) => {
    await mountComponents(page, {
      html: `<main id="home">${fixture.html}</main><div id="elsewhere"></div>`,
      components: [fixture.component],
      instrument: true,
    });
    const counts = await page.evaluate((tag) => {
      interface Bindings {
        readonly listeners: number;
        readonly observers: number;
      }
      const read = (globalThis as unknown as { __snBindings: () => Bindings }).__snBindings;
      const element = document.querySelector(tag);
      const home = document.getElementById("home");
      const elsewhere = document.getElementById("elsewhere");
      if (element === null || home === null || elsewhere === null)
        throw new Error("fixture_missing");
      const connected = read();
      interface AtomicMove {
        moveBefore?: (this: Element, node: Node, child: Node | null) => void;
      }
      const atomic = elsewhere as Element & AtomicMove;
      if (typeof atomic.moveBefore === "function") {
        (elsewhere as Element & Required<AtomicMove>).moveBefore(element, null);
        (home as Element & Required<AtomicMove>).moveBefore(element, null);
      }
      const movedAtomically = read();
      elsewhere.insertBefore(element, null);
      home.insertBefore(element, null);
      element.removeAttribute("data-sn-ready");
      element.removeAttribute("data-sn-upgraded");
      elsewhere.insertBefore(element, null);
      home.insertBefore(element, null);
      const reinserted = read();
      element.remove();
      const removed = read();
      return { connected, movedAtomically, reinserted, removed };
    }, fixture.element);
    expect(counts.connected.listeners, "the element binds on connection").toBeGreaterThan(0);
    expect(counts.movedAtomically, "an atomic move keeps the same bindings").toEqual(
      counts.connected,
    );
    expect(counts.reinserted, "reinsertion rebinds exactly once").toEqual(counts.connected);
    expect(counts.removed, "removal releases every binding").toEqual({
      listeners: 0,
      observers: 0,
    });
  });
}

test("UI-020: after a move, one click on the password reveal toggles once", async ({ page }) => {
  const fixture = FIXTURES.find((candidate) => candidate.component === "password-input");
  if (fixture === undefined) throw new Error("fixture_missing");
  await mountComponents(page, {
    html: `<main id="home">${fixture.html}</main><div id="elsewhere"></div>`,
    components: ["password-input"],
  });
  await page.evaluate(() => {
    const element = document.querySelector("sn-password-reveal");
    const elsewhere = document.getElementById("elsewhere");
    if (element === null || elsewhere === null) throw new Error("fixture_missing");
    elsewhere.appendChild(element);
  });
  await page.getByRole("button", { name: "Show" }).click();
  await expect(page.locator("#password")).toHaveAttribute("type", "text");
  await expect(page.getByRole("button", { name: "Hide" })).toHaveAttribute("aria-pressed", "true");
});
