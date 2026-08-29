import { expect, test } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";

for (const format of ["esm", "classic"] as const) {
  test(`${format} upload and async feedback remains accessible under strict CSP`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(
      `${REFERENCE_ORIGIN}/scenario/iteration004?features=both&format=${format}&transport=sse`,
    );
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    await expect(page.getByLabel("Iteration 004 file")).toHaveAccessibleName("Iteration 004 file");
    await expect(page.getByRole("status", { name: "Order updates" })).toContainText(
      "Updates current",
    );
    await expect(page.getByLabel("Iteration 004 file")).toHaveAttribute(
      "aria-describedby",
      "iteration-upload-error",
    );

    await page.getByRole("button", { name: "Toggle local details" }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByText("Local details are available")).toBeVisible();
    await expect(page.getByRole("button", { name: "Toggle local details" })).toBeFocused();
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.from("accessible-upload"),
      mimeType: "text/plain",
      name: "accessible.txt",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "ready",
    );
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-motion",
      "reduced",
    );
    await expectNoSeriousA11yViolations(page, {
      sourceUrl: `${REFERENCE_ORIGIN}/scenario/iteration004-axe.js`,
    });
    expect(
      await page.evaluate(async () => {
        const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
        const snapshot: unknown =
          (typeof probe === "object" || typeof probe === "function") && probe !== null
            ? Reflect.get(probe, "snapshot")
            : null;
        if (typeof snapshot !== "function") return ["probe_missing"];
        const invoke = snapshot as (this: unknown) => unknown;
        const value: unknown = await invoke.call(probe);
        if ((typeof value !== "object" && typeof value !== "function") || value === null) {
          return ["snapshot_invalid"];
        }
        const violations: unknown = Reflect.get(value, "cspViolations");
        return violations;
      }),
    ).toEqual([]);
  });
}

for (const artifact of ["missing", "incompatible"] as const) {
  for (const format of ["esm", "classic"] as const) {
    test(`${format} ${artifact} optional artifacts preserve native controls and focus`, async ({
      page,
    }, testInfo) => {
      test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
      await page.goto(
        `${REFERENCE_ORIGIN}/scenario/iteration004?features=both&format=${format}&artifact=${artifact}`,
      );
      await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
      await expect(page.getByRole("heading", { name: "Iteration 004 integration" })).toBeVisible();
      await page.getByRole("button", { name: "Native disclosure" }).focus();
      await page.keyboard.press("Enter");
      await expect(page.getByText("Native fallback details")).toBeVisible();
      await expect(page.getByRole("button", { name: "Native disclosure" })).toBeFocused();
      await expectNoSeriousA11yViolations(page, {
        sourceUrl: `${REFERENCE_ORIGIN}/scenario/iteration004-axe.js`,
      });
    });
  }
}
