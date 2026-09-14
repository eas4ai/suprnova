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
  await expectConnected(page, 3);

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
  await expectConnected(page, 3);
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
  await expectConnected(page, 3);
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
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await expectConnected(page, 3);
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
