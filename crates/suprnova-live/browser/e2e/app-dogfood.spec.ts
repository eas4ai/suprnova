import { expect, test, type Page } from "@playwright/test";

import { ISLAND_SELECTOR, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

const APP_ORIGIN = "http://127.0.0.1:4178";

async function expectConnected(page: Page, count: number): Promise<void> {
  const islands = page.locator(ISLAND_SELECTOR);
  await expect(islands).toHaveCount(count);
  for (let index = 0; index < count; index += 1) {
    await expect(islands.nth(index)).toHaveAttribute(STATUS_ATTRIBUTE, "connected");
  }
}

test("the public page renders server-side and its island connects for an anonymous visitor", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/public`);
  await expect(page.getByRole("heading", { name: "Public counter" })).toBeVisible();
  await expectConnected(page, 1);
  await expect(page.getByText("Count: 0", { exact: true })).toBeVisible();
});

test("a signed-in user runs actions through the production middleware stack", async ({ page }) => {
  const actions: number[] = [];
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname === "/__live/action") actions.push(response.status());
  });
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await expect(page.getByRole("heading", { name: "Live dashboard" })).toBeVisible();
  await expectConnected(page, 4);

  await page.getByRole("button", { name: "Increment" }).click();
  await expect(page.getByText("Count: 1", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Increment" }).click();
  await expect(page.getByText("Count: 2", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Reset" }).click();
  await expect(page.getByText("Count: 0", { exact: true })).toBeVisible();
  expect(actions.length).toBeGreaterThanOrEqual(3);
  expect(actions.every((status) => status === 200)).toBe(true);

  const html = await page.content();
  expect(html).toContain("suprnova-live.uploads.esm.js");
  expect(html).toContain("suprnova-live.async.esm.js");
  await expect(page.locator('input[type="file"]')).toHaveCount(1);
});

test("an anonymous visitor promotes the public island and increments it", async ({ page }) => {
  const statuses: number[] = [];
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname === "/__live/action") statuses.push(response.status());
  });
  await page.goto(`${APP_ORIGIN}/live/public`);
  await expectConnected(page, 1);
  await page.getByRole("button", { name: "Increment" }).click();
  await expect(page.getByText("Count: 1", { exact: true })).toBeVisible();
  expect(statuses).toEqual([200]);
});

test("the activity feed subscribes over the asynchronous transport and refreshes on a published event", async ({
  page,
}) => {
  const transports: string[] = [];
  const renders: number[] = [];
  const console: string[] = [];
  page.on("console", (message) => console.push(`${message.type()}: ${message.text()}`));
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/__live/async/")) transports.push(url.pathname);
  });
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname === "/__live/action") renders.push(response.status());
  });
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expectConnected(page, 4);
  const issued = await expect
    .poll(() => transports.some((path) => path === "/__live/async/subscriptions"))
    .toBe(true)
    .then(
      () => true,
      () => false,
    );
  if (!issued) {
    throw new Error(
      `no subscription issued; transports=${transports.join(",")}; console=${console.join(" | ")}`,
    );
  }
  await expect
    .poll(() =>
      transports.some((path) => path === "/__live/async/events" || path === "/__live/async/socket"),
    )
    .toBe(true);
  const before = renders.length;
  // The server counts posts for its whole lifetime, across engine projects.
  const postedBefore = Number(await page.locator("[data-posted]").getAttribute("data-posted"));
  const posted = await page.request.get(`${APP_ORIGIN}/live/demo-post`);
  expect(posted.status()).toBe(200);
  await expect.poll(() => renders.length).toBeGreaterThan(before);
  expect(renders.every((status) => status === 200)).toBe(true);
  // The delivered refresh is visible in the island, not only in the log: the
  // fresh render shows the post the server recorded.
  const expected = String(postedBefore + 1);
  await expect(page.locator("[data-posted]")).toHaveAttribute("data-posted", expected);
  await expect(page.getByText(`Posted ${expected}`, { exact: true })).toBeVisible();
  await expectConnected(page, 4);
});

test("the form gallery's save form submits through Live with the page left in place", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/forms`);
  await expectConnected(page, 1);
  const statuses: number[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/__live/action")) statuses.push(response.status());
  });
  let navigations = 0;
  page.on("framenavigated", (frame) => {
    if (frame === page.mainFrame()) navigations += 1;
  });
  // The required controls first, so the browser's own validation lets the
  // submit event through; then live:submit.prevent runs the Live action and
  // the native GET submission, which would reload the page from the form's
  // own query, does not.
  await page.locator("#email").fill("ada@example.com");
  await page.locator("#secret").fill("correct horse battery staple");
  await page.locator("#agree").check();
  // The model proposals those controls send settle before the submit, so the
  // requests counted from here are the submit's own.
  await expectConnected(page, 1);
  const beforeSubmit = statuses.length;
  await page.getByRole("button", { name: "Save" }).click();
  await expect.poll(() => statuses.length).toBeGreaterThan(beforeSubmit);
  expect(statuses.every((status) => status === 200)).toBe(true);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/forms`);
  await expectConnected(page, 1);
  expect(navigations).toBe(0);
});

test("the form gallery loads the suprnova-ui base and its vendored assets, and the password reveal upgrades", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await page.goto(`${APP_ORIGIN}/live/forms`);
  await expect(page.getByRole("heading", { name: "Form gallery" })).toBeVisible();
  await expectConnected(page, 1);

  const stylesheets = await page.evaluate(() =>
    [...document.querySelectorAll('link[rel="stylesheet"]')].map(
      (link) => (link as HTMLLinkElement).href,
    ),
  );
  expect(
    stylesheets.some(
      (href) => href.includes("/__live/assets/") && href.endsWith("/suprnova-ui.css"),
    ),
  ).toBe(true);
  expect(stylesheets.some((href) => href.endsWith("/suprnova-ui/field/field.css"))).toBe(true);
  const layered = await page.evaluate(() =>
    [...document.styleSheets].some((sheet) => {
      try {
        return [...sheet.cssRules].some(
          (rule) => rule instanceof CSSLayerBlockRule && rule.name === "suprnova-ui",
        );
      } catch {
        return false;
      }
    }),
  );
  expect(layered).toBe(true);

  // UI-012, UI-018: the password reveal is a light-DOM element defined only
  // by its own vendored script, upgraded on this engine.
  await expect
    .poll(() => page.evaluate(() => customElements.get("sn-password-reveal") !== undefined))
    .toBe(true);
  const password = page.locator("#secret");
  await expect(password).toHaveAttribute("type", "password");
  const reveal = page.getByRole("button", { name: "Show" });
  await expect(reveal).toBeVisible();
  await reveal.click();
  await expect(password).toHaveAttribute("type", "text");
  await expect(page.getByRole("button", { name: "Hide" })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Hide" }).click();
  await expect(password).toHaveAttribute("type", "password");
  const shadowRoots = await page.evaluate(
    () =>
      [...document.querySelectorAll("*")].filter((element) => element.shadowRoot !== null).length,
  );
  expect(shadowRoots).toBe(0);
});

test("a document that never added a library component defines no sn- element", async ({ page }) => {
  // The dashboard mounts the account menu now, so the public page is the
  // document without a library component.
  await page.goto(`${APP_ORIGIN}/live/public`);
  await expect(page.getByRole("heading", { name: "Public counter" })).toBeVisible();
  await expectConnected(page, 1);
  const defined = await page.evaluate(() => customElements.get("sn-password-reveal") !== undefined);
  expect(defined).toBe(false);
  const prefixed = await page.evaluate(() =>
    [...document.querySelectorAll("*")].some((element) =>
      element.tagName.toLowerCase().startsWith("sn-"),
    ),
  );
  expect(prefixed).toBe(false);
  const html = await page.content();
  expect(html).not.toContain("suprnova-ui.css");
});

test("the overlay gallery opens and closes every overlay without a Live request, and a dialog returns focus to its invoker", async ({
  page,
}) => {
  const actions: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/__live/action") actions.push(request.method());
  });
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await page.goto(`${APP_ORIGIN}/live/overlays`);
  await expect(page.getByRole("heading", { name: "Overlay gallery" })).toBeVisible();
  await expectConnected(page, 1);
  const defined = await page.evaluate(() =>
    ["sn-dialog", "sn-sheet", "sn-drawer"].map((name) => customElements.get(name) !== undefined),
  );
  expect(defined).toEqual([true, true, true]);

  // Popover and menu: native open state and light dismiss.
  await page.getByRole("button", { name: "Hint" }).click();
  await expect(page.locator("#hint")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator("#hint")).toBeHidden();
  await page.getByRole("button", { name: "Actions" }).click();
  await expect(page.locator("#actions")).toBeVisible();
  await expect(page.locator("#actions a", { hasText: "Dashboard" })).toHaveAttribute(
    "href",
    "/live",
  );
  await page.keyboard.press("Escape");
  await expect(page.locator("#actions")).toBeHidden();

  // Disclosure and accordion.
  const notes = page.locator("details.sn-collapsible");
  await notes.locator("summary").click();
  await expect(notes).toHaveAttribute("open", "");
  await expect(page.locator("details.sn-accordion-item").first()).toHaveAttribute("open", "");

  // Dialog, sheet and drawer: focus containment and return.
  for (const [trigger, dialogId] of [
    ["Delete everything", "confirm"],
    ["Details", "details-sheet"],
    ["Navigate", "nav-drawer"],
  ] as const) {
    const button = page.getByRole("button", { name: trigger });
    await button.click();
    const dialog = page.locator(`dialog#${dialogId}`);
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute("open", "");
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(button).toBeFocused();
  }
  expect(actions).toEqual([]);

  // The one server effect an overlay invokes owns only that effect.
  await page.getByRole("button", { name: "Delete everything" }).click();
  await page.locator("dialog#confirm").getByRole("button", { name: "Delete" }).click();
  await expect(page.locator("[data-deleted]")).toHaveAttribute("data-deleted", "true");
  await expect(page.locator("dialog#confirm")).toBeVisible();
  await page.locator("dialog#confirm").getByRole("button", { name: "Cancel" }).click();
  await expect(page.locator("dialog#confirm")).toBeHidden();
  expect(actions).toEqual(["POST"]);
});

test("an open keyed overlay survives a morph that did not touch it, and focus falls back when the invoker left", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/overlays`);
  await expectConnected(page, 1);
  const notes = page.locator("details.sn-collapsible");
  await notes.locator("summary").click();
  await expect(notes).toHaveAttribute("open", "");
  await expect(page.locator("[data-notes]")).toHaveAttribute("data-notes", "1");
  await page.getByRole("button", { name: "Actions" }).click();
  await page.locator("#actions").getByRole("button", { name: "Add a note" }).click();
  await expect(page.locator("[data-notes]")).toHaveAttribute("data-notes", "2");
  await expect(notes).toHaveAttribute("open", "");
  await expect(notes.getByText("Note 2", { exact: true })).toBeVisible();
  await expect(page.locator("#actions")).toBeHidden();

  await page.getByRole("button", { name: "Details" }).click();
  await expect(page.locator("dialog#details-sheet")).toBeVisible();
  await page.evaluate(() => {
    document.querySelector('[data-sn-sheet-open="details-sheet"]')?.remove();
  });
  await page.keyboard.press("Escape");
  await expect(page.locator("dialog#details-sheet")).toBeHidden();
  await expect(page.locator("sn-sheet")).toBeFocused();
});

test("a toast announces once from the status region without moving focus, and a critical error also renders as an alert", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/feedback`);
  await expect(page.getByRole("heading", { name: "Feedback gallery" })).toBeVisible();
  await expectConnected(page, 1);
  expect(await page.evaluate(() => customElements.get("sn-toast-region") !== undefined)).toBe(true);
  const region = page.locator("#toasts");
  await expect(region).toHaveAttribute("role", "status");
  await expect(region).toHaveAttribute("aria-live", "polite");

  const save = page.getByRole("button", { name: "Save" });
  await save.click();
  await expect(region.locator(".sn-toast")).toHaveCount(1);
  await expect(region.getByText("Saved 1 times", { exact: true })).toBeVisible();
  await expect(save).toBeFocused();
  await expect(page.locator("[data-saved]")).toHaveAttribute("data-saved", "1");

  // A critical error is never only a toast: the persistent alert renders too.
  await page.getByRole("button", { name: "Fail" }).click();
  const failure = page.locator("#failure");
  await expect(failure).toBeVisible();
  await expect(failure).toHaveAttribute("role", "alert");
  await expect(region.locator(".sn-toast")).toHaveCount(2);

  // Dismissal is the browser's, and a morph keeps a dismissed toast dismissed.
  const first = region.locator(".sn-toast").first();
  await first.getByRole("button", { name: "Dismiss" }).click();
  await expect(first).toBeHidden();
  await page.getByRole("button", { name: "Advance" }).click();
  await expect(page.locator("#upload")).toHaveAttribute("value", "25");
  await expect(first).toBeHidden();
  await expect(region.locator(".sn-toast").nth(1)).toBeVisible();
});

test("the flash region shows an outcome once after a redirect and nothing on the next document", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  // The dashboard's islands connect through requests that carry the session,
  // and every request that loads the session ages the flash, so one landing
  // after the notice would consume it: wait until they are all connected.
  // Session blocking (SESS-001) is on for every route here, so the notice's
  // write can no longer lose to one of theirs either.
  await expectConnected(page, 4);
  await page.goto(`${APP_ORIGIN}/live/feedback/notice`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/feedback`);
  await expect(page.locator("#flash .sn-flash")).toHaveText("Your changes were saved");
  await page.reload();
  await expect(page.locator("#flash .sn-flash")).toHaveCount(0);
});

test("route pagination links canonical pages, and Live pagination reflects the page into the query without a history entry", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/navigation`);
  await expect(page.getByRole("heading", { name: "Navigation gallery" })).toBeVisible();
  await expectConnected(page, 1);

  const routeLinks = page.locator("#route-pages a");
  await expect(routeLinks).toHaveCount(4);
  await expect(routeLinks.nth(1)).toHaveAttribute("href", "/live/navigation?page=2");
  await expect(page.locator('#route-pages [aria-current="page"]')).toHaveText("1");

  const length = await page.evaluate(() => history.length);
  await page.locator("#live-pages").getByRole("button", { name: "Next" }).click();
  await expect(page.locator("[data-page]")).toHaveAttribute("data-page", "2");
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/navigation?page=2`);
  expect(await page.evaluate(() => history.length)).toBe(length);

  // The reflected query reloads onto the same page.
  await page.reload();
  await expect(page.locator("[data-page]")).toHaveAttribute("data-page", "2");
  await page.locator("#live-pages").getByRole("button", { name: "Previous" }).click();
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/navigation?page=1`);
  expect(await page.evaluate(() => history.length)).toBe(length);

  // Local tabs change without a request; the sidebar group keeps its open state across a morph.
  const requests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/__live/action") requests.push(request.method());
  });
  await page.getByRole("tab", { name: "History" }).click();
  await expect(page.getByRole("tab", { name: "History" })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("#panel-history")).toBeVisible();
  await expect(page.locator("#panel-summary")).toBeHidden();
  expect(requests).toEqual([]);

  // Load more appends keyed rows and keeps the ones already there; the control leaves on the last page.
  await expect(page.locator("#feed li")).toHaveCount(3);
  // A property on the node, not an attribute: the morph syncs attributes from the
  // server render, so only a node the morph kept still carries the witness.
  const firstRow = page.locator("#feed li").first();
  const before = await firstRow.evaluate((node) => {
    (node as HTMLElement & { witness?: string }).witness = "kept";
    return node.textContent;
  });
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(page.locator("#feed li")).toHaveCount(6);
  expect(
    await page
      .locator("#feed li")
      .first()
      .evaluate((node) => (node as HTMLElement & { witness?: string }).witness),
  ).toBe("kept");
  expect(await page.locator("#feed li").first().textContent()).toBe(before);
  await expect(page.getByRole("tab", { name: "History" })).toHaveAttribute("aria-selected", "true");
  await page.getByRole("button", { name: "Load more" }).click();
  await expect(page.locator("#feed li")).toHaveCount(9);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);
  expect(requests).toEqual(["POST", "POST"]);
});

test("the data-display gallery keeps one focus order across viewports, and a reorder keeps every keyed item", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/data-display`);
  await expect(page.getByRole("heading", { name: "Data display gallery" })).toBeVisible();
  await expectConnected(page, 2);

  // DATA-001: tabbing through the gallery visits the same controls in the same
  // order at a phone width and at a desktop width; the layout never reorders.
  const focusOrder = async () => {
    await page.locator("body").click({ position: { x: 1, y: 1 } });
    const seen: string[] = [];
    for (let step = 0; step < 12; step += 1) {
      await page.keyboard.press("Tab");
      const label = await page.evaluate(() => {
        const active = document.activeElement as HTMLElement | null;
        if (active === null || active === document.body) return "body";
        const name =
          active.id !== ""
            ? active.id
            : (active.getAttribute("aria-label") ?? active.textContent.trim());
        return `${active.tagName.toLowerCase()}:${name}`;
      });
      seen.push(label);
    }
    return seen;
  };
  await page.setViewportSize({ width: 400, height: 800 });
  const narrow = await focusOrder();
  await page.setViewportSize({ width: 1280, height: 800 });
  const wide = await focusOrder();
  expect(wide).toEqual(narrow);
  expect(narrow).toContain("div:activity-scroll");

  // DATA-002: every badge and stat names its status in text.
  await expect(page.locator("[data-sn-trend='up'] .sn-stat-direction")).toHaveText("Up");
  await expect(page.locator("[data-sn-trend='down'] .sn-stat-direction")).toHaveText("Down");
  await expect(page.locator(".sn-badge").first()).not.toBeEmpty();

  // DATA-003: a reorder keeps the node of every keyed item.
  const firstItem = page.locator("#activity li").first();
  const before = await firstItem.evaluate((node) => {
    (node as HTMLElement & { witness?: string }).witness = "kept";
    return node.getAttribute("live:key");
  });
  await page.getByRole("button", { name: "Reorder" }).click();
  await expect(page.locator("#activity li").last()).toHaveAttribute("live:key", before ?? "");
  expect(
    await page
      .locator("#activity li")
      .last()
      .evaluate((node) => (node as HTMLElement & { witness?: string }).witness),
  ).toBe("kept");

  // DATA-004: the chart marks are server-rendered SVG with a data table beside them.
  await expect(page.locator("#revenue-chart svg")).toHaveCount(1);
  await expect(page.locator("#revenue-chart details table")).toHaveCount(1);
});

test("the datatable sorts, filters and pages through the URL without a history entry", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/data-display`);
  await expectConnected(page, 2);
  const table = page.locator("#invoices");
  await expect(table.locator("caption")).toContainText("Invoices");
  await expect(table.locator("thead th[scope='col']")).toHaveCount(4);
  await expect(table.locator("tbody tr")).toHaveCount(4);

  const length = await page.evaluate(() => history.length);
  await table.getByRole("button", { name: /Amount/ }).click();
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/data-display?sort=amount`);
  await expect(table.locator("th[aria-sort='ascending']")).toHaveCount(1);
  await table.getByRole("button", { name: /Amount/ }).click();
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/data-display?dir=desc&sort=amount`);
  await expect(table.locator("th[aria-sort='descending']")).toHaveCount(1);
  expect(await page.evaluate(() => history.length)).toBe(length);

  await page.locator("#invoices-filter").fill("acme");
  await page.getByRole("button", { name: "Apply" }).click();
  await expect(page).toHaveURL(`${APP_ORIGIN}/live/data-display?dir=desc&filter=acme&sort=amount`);
  await expect(table.locator("tbody tr")).toHaveCount(2);

  // The shared URL renders the same view.
  await page.reload();
  await expect(table.locator("th[aria-sort='descending']")).toHaveCount(1);
  await expect(page.locator("#invoices-filter")).toHaveValue("acme");
  await expect(table.locator("tbody tr")).toHaveCount(2);
});

test("the live-native enhancements upgrade, and every control still submits with its script blocked", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await page.goto(`${APP_ORIGIN}/live/live-native`);
  await expect(page.getByRole("heading", { name: "Live native gallery" })).toBeVisible();
  await expectConnected(page, 2);
  const island = page.locator(
    "[data-suprnova-live-island][data-suprnova-live-document-key='live-native-gallery']",
  );
  const status = page.locator("#activity [data-live-stream-status]");
  await expect(island).toHaveAttribute("data-live-stream-state", /current|connecting|degraded/);

  // FORM-006: the element mirrors the code into the cells; the input holds it.
  await expect(page.locator("sn-input-otp")).toHaveAttribute("data-sn-upgraded", "");
  const code = page.locator("#code");
  await code.fill("246810");
  await expect(page.locator(".sn-otp-cell[data-sn-index='5']")).toHaveText("0");
  await page.locator("#code-form button[type=submit]").click();
  await expect(page.locator("[data-verified='1']")).toBeVisible();
  // FDB-005: the action's morph keeps the runtime's status; the server's
  // disconnected default never shows over a projected stream state.
  await expect(island).toHaveAttribute("data-live-stream-state", /current|connecting|degraded/);
  await expect(status).not.toHaveText("Updates disconnected");

  // FORM-007: the strips are radios; choosing all three composes the input.
  await page.locator("#renewal-form summary").click();
  // The radios are visually hidden, so the labels take the clicks.
  await page
    .locator("[data-sn-part='year'] label")
    .filter({ hasText: /^2027$/ })
    .click();
  await page
    .locator("[data-sn-part='month'] label")
    .filter({ hasText: /^March$/ })
    .click();
  await page.locator("[data-sn-part='day'] label").filter({ hasText: /^9$/ }).click();
  await expect(page.locator("#when")).toHaveValue("2027-03-09");

  // FORM-008: the listbox opens with arrow keys, the active option moves, Enter selects.
  const country = page.locator("#country");
  await expect(page.locator("sn-combobox")).toHaveAttribute("data-sn-upgraded", "");
  await country.fill("ca");
  await expect(country).toHaveAttribute("aria-expanded", "true");
  // The results for "ca" arrive through the model round-trip; the selection
  // is made from them, as a user would, not from the seed list.
  await expect(page.locator("#country-listbox")).toHaveAttribute("data-sn-query", "ca");
  await country.press("ArrowDown");
  await expect(country).toHaveAttribute("aria-activedescendant", "country-option-1");
  await country.press("Enter");
  await expect(country).toHaveValue("Canada");
  await expect(country).toHaveAttribute("aria-expanded", "false");

  // FDB-005: the stream connects and the status names the state honestly.
  await expect(island).toHaveAttribute("data-live-stream-state", /current|connecting|degraded/);
  await expect(status).not.toHaveText("");
  const state = await island.getAttribute("data-live-stream-state");
  if (state !== "current") {
    await expect(status).not.toHaveText(/current/i);
  }
  await page.getByRole("button", { name: "Post an update" }).click();
  await expect(page.locator(".sn-bell-count")).toHaveText(/1 unread/);
  await expect(page.locator(".sn-live-feed-item").first()).toHaveText(/Update \d+ posted/);

  // NAV-005: the account menu is a details disclosure on its own island.
  await page.locator("#account summary").click();
  await expect(page.locator("#account").getByRole("link", { name: "Profile" })).toBeVisible();

  // With every element script blocked, the native controls still carry the values.
  await page.route("**/suprnova-ui/**/*.js", (route) => route.abort());
  await page.goto(`${APP_ORIGIN}/live/live-native`);
  await expectConnected(page, 2);
  expect(await page.evaluate(() => customElements.get("sn-input-otp"))).toBeUndefined();
  expect(await page.evaluate(() => customElements.get("sn-combobox"))).toBeUndefined();
  await page.locator("#code").fill("135791");
  await page.locator("#code-form button[type=submit]").click();
  await expect(page.locator("[data-verified='1']")).toBeVisible();
  // A strip selection needs no script; the date input takes the date itself.
  await page.locator("#renewal-form summary").click();
  await page.locator("[data-sn-part='day'] label").filter({ hasText: /^2$/ }).click();
  await expect(page.locator("input[name='when-day'][value='2']")).toBeChecked();
  await page.locator("#when").fill("2026-06-15");
  await page.locator("#country").fill("Chile");
  await page.locator("#renewal-form button[type=submit]").click();
  await expect(page.locator("[data-when='2026-06-15']")).toBeVisible();
  await page.unroute("**/suprnova-ui/**/*.js");
});

test("a proposal typed while its predecessor is in flight keeps the island's authority and never reloads", async ({
  page,
}) => {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await page.goto(`${APP_ORIGIN}/live/live-native`);
  await expectConnected(page, 2);
  const island = page.locator(
    "[data-suprnova-live-island][data-suprnova-live-document-key='live-native-gallery']",
  );
  const statuses: number[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/__live/action")) statuses.push(response.status());
  });
  let navigations = 0;
  page.on("framenavigated", (frame) => {
    if (frame === page.mainFrame()) navigations += 1;
  });
  // The first proposal is held in flight until the second one is queued
  // behind it; the runtime must apply the first response's authority and
  // send the second against it, never against the consumed snapshot.
  const gate: { release: (() => void) | null; held: boolean } = { held: false, release: null };
  await page.route("**/__live/action", async (route) => {
    if (!gate.held) {
      gate.held = true;
      await new Promise<void>((resolve) => {
        gate.release = resolve;
      });
    }
    await route.continue();
  });
  const country = page.locator("#country");
  await country.fill("ca");
  await expect.poll(() => gate.held).toBe(true);
  await country.fill("Canada");
  // The gallery shows the field's queued state; the second proposal now
  // sits behind the held one.
  await expect(page.getByText("Search pending", { exact: true })).toBeVisible();
  gate.release?.();
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "2");
  await expect(page.locator("#country-listbox")).toHaveAttribute("data-sn-query", "Canada");
  await expect(country).toHaveValue("Canada");
  expect(statuses).toEqual([200, 200]);
  expect(navigations).toBe(0);
  await page.unroute("**/__live/action");
});
