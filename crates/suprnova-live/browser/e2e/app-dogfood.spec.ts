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
    if (url.pathname === "/__live/v1/action") actions.push(response.status());
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

test("an anonymous visitor cannot run an action on the public island", async ({ page }) => {
  const statuses: number[] = [];
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname === "/__live/v1/action") statuses.push(response.status());
  });
  await page.goto(`${APP_ORIGIN}/live/public`);
  await expectConnected(page, 1);
  await page.getByRole("button", { name: "Increment" }).click();
  await expect.poll(() => statuses.length).toBeGreaterThanOrEqual(1);
  expect(statuses[0]).toBe(401);
  await expect(page.getByText("Count: 0", { exact: true })).toBeVisible();
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
    if (url.pathname.startsWith("/__live/v1/async/")) transports.push(url.pathname);
  });
  page.on("response", (response) => {
    const url = new URL(response.url());
    if (url.pathname === "/__live/v1/action") renders.push(response.status());
  });
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expectConnected(page, 3);
  const issued = await expect
    .poll(() => transports.some((path) => path === "/__live/v1/async/subscriptions"))
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
      transports.some(
        (path) => path === "/__live/v1/async/events" || path === "/__live/v1/async/socket",
      ),
    )
    .toBe(true);
  const before = renders.length;
  const posted = await page.request.get(`${APP_ORIGIN}/live/demo-post`);
  expect(posted.status()).toBe(200);
  await expect.poll(() => renders.length).toBeGreaterThan(before);
  expect(renders.every((status) => status === 200)).toBe(true);
  await expectConnected(page, 3);
});
