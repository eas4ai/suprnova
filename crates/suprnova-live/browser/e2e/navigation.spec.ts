import { expect, test } from "@playwright/test";

test("anchors, redirects, refresh, fragments, errors, and Back/Forward remain complete navigation", async ({
  page,
}) => {
  await page.goto("/scenario/navigation");
  const firstToken = await page.locator("html").getAttribute("data-document-token");

  await page.locator("#ordinary-link").click();
  await expect(page).toHaveURL(/\/scenario\/navigationDestination$/u);
  await expect(page.locator("#destination-marker")).toHaveText("Complete canonical destination");
  await expect(page.locator("#destination-focus")).toBeFocused();

  await page.goBack();
  await expect(page).toHaveURL(/\/scenario\/navigation$/u);
  await expect(page.locator("#source-marker")).toHaveText("Complete source document");
  await page.goForward();
  await expect(page.locator("#destination-marker")).toBeVisible();
  await page.goBack();

  await page.locator("#redirect-link").click();
  await expect(page).toHaveURL(/redirected=1/u);
  await page.goBack();

  await page.locator("#error-link").click();
  await expect(page.locator("h1")).toHaveText("Not found");
  expect((await page.request.get(page.url())).status()).toBe(404);
  await page.goBack();

  await page.locator("#fragment-link").click();
  await expect(page).toHaveURL(/#fragment-target$/u);
  await expect(page.locator("#fragment-target")).toBeFocused();
  await page.goBack();

  await page.reload();
  const refreshedToken = await page.locator("html").getAttribute("data-document-token");
  expect(refreshedToken).not.toBe(firstToken);
});

test("GET/POST forms and modifier/new-tab/download semantics stay native", async ({
  page,
  context,
}) => {
  await page.goto("/scenario/navigation");

  await page.locator("#get-form button").click();
  await expect(page).toHaveURL(/\/scenario\/navigationDestination\?query=forms$/u);
  await page.goBack();

  await page.locator("#post-form button").click();
  await expect(page).toHaveURL(/\/navigation\/post$/u);
  await expect(page.locator("#post-body")).toContainText("message=posted");
  await page.goBack();

  const popupPromise = context.waitForEvent("page");
  await page.locator("#new-tab-link").click();
  const popup = await popupPromise;
  await popup.waitForLoadState("domcontentloaded");
  await expect(popup.locator("#destination-marker")).toBeVisible();
  await popup.close();
  await expect(page).toHaveURL(/\/scenario\/navigation$/u);

  await expect(page.locator("#download-link")).toHaveAttribute("download", "report.txt");
  await expect(page.locator("#external-link")).toHaveAttribute("href", "https://example.invalid/");
});

test("dirty guard offers stay with focus return and a guaranteed leave path", async ({ page }) => {
  await page.goto("/scenario/navigation");
  await page.locator("#dirty-input").fill("unsaved");

  page.once("dialog", async (dialog) => {
    expect(dialog.message()).toBe("Discard the unsaved navigation draft?");
    await dialog.dismiss();
  });
  await page.locator("#guarded-link").click();
  await expect(page).toHaveURL(/\/scenario\/navigation$/u);
  await expect(page.locator("#guarded-link")).toBeFocused();

  page.once("dialog", async (dialog) => {
    await dialog.accept();
  });
  await page.locator("#guarded-link").click();
  await expect(page).toHaveURL(/guarded=1/u);
});

test("prefetch emits only an eligible native resource and never installs its body", async ({
  page,
}) => {
  await page.goto("/scenario/navigation");

  await expect(
    page.locator(
      'link[data-suprnova-live-prefetch-resource][href$="/scenario/navigationPrefetch"]',
    ),
  ).toHaveCount(1);
  await expect(
    page.locator('link[data-suprnova-live-prefetch-resource][href$="/scenario/navigationPrivate"]'),
  ).toHaveCount(0);
  await expect(
    page.locator('link[data-suprnova-live-prefetch-resource][href$="/scenario/navigationHidden"]'),
  ).toHaveCount(0);
  await expect(page.locator("#source-marker")).toHaveText("Complete source document");
  await expect(page.locator("#destination-marker")).toHaveCount(0);
});
