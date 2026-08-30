import { expect, test, type Page } from "@playwright/test";

const FEATURE_SYMBOL_KEY = "suprnova.live.features.v1";

async function installIncompatibleAsyncArtifact(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const driver = Object.freeze([
      Symbol.for("suprnova.live.feature-driver.v1"),
      1,
      0,
      Object.freeze({}),
      () => true,
    ]);
    const surface = {
      configureAsync() {
        // The incompatible artifact never receives runtime authority.
      },
      register() {
        return "incompatible";
      },
      version: 1,
    };
    Object.defineProperty(surface, Symbol.for("suprnova.live.features.v1.adopt"), {
      configurable: false,
      enumerable: false,
      value: () => driver,
      writable: false,
    });
    Object.freeze(surface);
    Object.defineProperty(window, Symbol.for("suprnova.live.features.v1"), {
      configurable: false,
      enumerable: false,
      value: surface,
      writable: false,
    });
  });
}

async function expectOrdinaryLiveAction(page: Page): Promise<void> {
  await page.goto("/scenario/fullFlow");
  const liveRequest = page.waitForRequest((request) => new URL(request.url()).pathname === "/live");
  await page.locator("#flow-action").click();
  const request = await liveRequest;
  const body = request.postDataJSON() as Readonly<Record<string, unknown>>;

  expect(body["operations"]).toEqual([
    { field: "query", kind: "sync_model" },
    { arguments: {}, kind: "invoke_action", name: "search" },
  ]);
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  await expect(page).toHaveURL(/state=done/u);
}

test("ordinary HTTP Live actions remain available without the optional async artifact", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));

  await expectOrdinaryLiveAction(page);

  expect(
    await page.evaluate(
      (symbol) => Reflect.get(window, Symbol.for(symbol)) !== undefined,
      FEATURE_SYMBOL_KEY,
    ),
  ).toBe(false);
  expect(errors).toEqual([]);
});

test("ordinary HTTP Live actions remain available when the async driver is incompatible", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await installIncompatibleAsyncArtifact(page);

  await expectOrdinaryLiveAction(page);

  expect(
    await page.evaluate((symbol) => {
      const surface: unknown = Reflect.get(window, Symbol.for(symbol));
      if ((typeof surface !== "object" && typeof surface !== "function") || surface === null) {
        return null;
      }
      const version: unknown = Reflect.get(surface, "version");
      return typeof version === "number" ? version : null;
    }, FEATURE_SYMBOL_KEY),
  ).toBe(1);
  expect(errors).toEqual([]);
});
