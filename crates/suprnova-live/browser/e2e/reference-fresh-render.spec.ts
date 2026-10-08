import { expect, test } from "@playwright/test";

const SCENARIO = "http://127.0.0.1:4175/scenario/referenceFreshRender";

test("production browser eligibility commits the Rust fresh-render successor", async ({
  browserName,
  page,
}) => {
  test.skip(browserName !== "chromium", "One physical production-browser proof is sufficient.");
  await page.goto(SCENARIO);

  const island = page.locator("[data-suprnova-live-island]");
  await expect(island).toHaveAttribute("data-suprnova-live-component", "reference.uploads");
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "1");
  await expect(island).toHaveAttribute("data-suprnova-live-protocol-min", "1");
  await expect(island).not.toHaveAttribute("live:poll.immediate", "");
  await expect(island.locator("[data-live-poll-generation]")).toHaveAttribute(
    "data-live-poll-generation",
    "1",
  );
  await expect
    .poll(() =>
      page.evaluate(() => {
        const evidence: unknown = Reflect.get(window, "__suprnovaFreshRender");
        if (typeof evidence !== "object" || evidence === null) return null;
        const acceptedRevision: unknown = Reflect.get(evidence, "acceptedRevision");
        const requests: unknown = Reflect.get(evidence, "requests");
        return {
          acceptedRevision,
          requests,
        };
      }),
    )
    .toEqual({ acceptedRevision: "1", requests: 1 });
});
