import { expect, test } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";

test.beforeEach(async ({ request }) => {
  await expect
    .poll(async () => {
      const response = await request.get(`${REFERENCE_ORIGIN}/__test/iteration-004/inspection`);
      if (!response.ok()) return null;
      const value = (await response.json()) as {
        readonly active_physical_transports: number;
        readonly active_uploads: number;
        readonly logical_memberships: number;
        readonly paused_upload_operations: number;
      };
      return {
        memberships: value.logical_memberships,
        paused: value.paused_upload_operations,
        physical: value.active_physical_transports,
        uploads: value.active_uploads,
      };
    })
    .toEqual({ memberships: 0, paused: 0, physical: 0, uploads: 0 });
  const reset = await request.post(
    `${REFERENCE_ORIGIN}/__test/iteration-004/control/upload/reset-creation-window`,
  );
  expect(reset.status()).toBe(204);
});

async function probeCommand(page: import("@playwright/test").Page, name: string): Promise<void> {
  await page.evaluate(async (commandName) => {
    const probe: unknown = Reflect.get(window, "__suprnovaIteration004");
    const callback: unknown =
      (typeof probe === "object" || typeof probe === "function") && probe !== null
        ? Reflect.get(probe, commandName)
        : null;
    if (typeof callback !== "function") throw new Error(`iteration_004_${commandName}_missing`);
    await Reflect.apply(callback, probe, []);
  }, name);
}

async function hostResources(page: import("@playwright/test").Page): Promise<{
  readonly active_uploads: number;
  readonly paused_upload_operations: number;
}> {
  return page.evaluate(async () => {
    const response = await fetch("/__test/iteration-004/inspection", { cache: "no-store" });
    if (!response.ok) throw new Error("iteration_004_inspection_failed");
    return response.json() as Promise<{
      readonly active_uploads: number;
      readonly paused_upload_operations: number;
    }>;
  });
}

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
    await page.getByRole("button", { name: "Remove upload" }).click();
    await expect.poll(async () => (await hostResources(page)).active_uploads).toBe(0);
  });
}

for (const format of ["esm", "classic"] as const) {
  test(`${format} upload cancel retry and remove are keyboard operable with stable focus`, async ({
    context,
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await page.goto(`${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}`);
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");

    const input = page.getByLabel("Iteration 004 file");
    await input.setInputFiles({
      buffer: Buffer.from("cancel-by-keyboard"),
      mimeType: "text/plain",
      name: "cancel.txt",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "ready",
    );
    const cancel = page.getByRole("button", { name: "Cancel upload" });
    await cancel.focus();
    await page.keyboard.press("Space");
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "canceled",
    );
    const remove = page.getByRole("button", { name: "Remove upload" });
    await expect(remove).toBeFocused();
    await expect.poll(async () => (await hostResources(page)).active_uploads).toBe(0);

    await page.keyboard.press("Enter");
    await expect(input).toBeFocused();
    expect(
      await input.evaluate((element) => {
        return element instanceof HTMLInputElement ? element.files?.length : -1;
      }),
    ).toBe(0);

    await context.setOffline(true);
    await input.setInputFiles({
      buffer: Buffer.from("retry-by-keyboard"),
      mimeType: "text/plain",
      name: "retry.txt",
    });
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "interrupted",
    );
    const retry = page.getByRole("button", { name: "Retry upload" });
    await retry.focus();
    await context.setOffline(false);
    await page.keyboard.press("Enter");
    await expect(page.locator("#iteration-upload-progress")).toHaveAttribute(
      "data-live-upload-state",
      "ready",
    );
    await expect(cancel).toBeFocused();

    await remove.focus();
    await page.keyboard.press("Space");
    await expect(input).toBeFocused();
    expect(
      await input.evaluate((element) => {
        return element instanceof HTMLInputElement ? element.files?.length : -1;
      }),
    ).toBe(0);
    await expect.poll(async () => (await hostResources(page)).active_uploads).toBe(0);
  });

  test(`${format} multi-chunk progress coalesces numeric live announcements on a controlled clock`, async ({
    page,
  }, testInfo) => {
    test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
    await page.goto(
      `${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=${format}&controlled-upload-clock=true`,
    );
    await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
    await probeCommand(page, "pauseEveryUploadChunk");
    await page.getByLabel("Iteration 004 file").setInputFiles({
      buffer: Buffer.alloc(3 * 256 * 1024, 0x61),
      mimeType: "application/octet-stream",
      name: "three-chunks.bin",
    });
    const progress = page.locator("#iteration-upload-progress");
    await expect.poll(async () => (await hostResources(page)).paused_upload_operations).toBe(1);
    await expect(progress).toHaveAttribute("data-live-upload-percent", "0");
    await expect(progress).toHaveAttribute("aria-valuenow", "0");

    await probeCommand(page, "resumePausedUpload");
    await expect.poll(async () => (await hostResources(page)).paused_upload_operations).toBe(1);
    await expect(progress).toHaveAttribute("data-live-upload-percent", "33");
    await expect(progress).toHaveAttribute("aria-valuenow", "0");

    await page.evaluate(() => {
      const probe = Reflect.get(window, "__suprnovaIteration004") as {
        advanceUploadClock(milliseconds: number): void;
      };
      probe.advanceUploadClock(499);
    });
    await probeCommand(page, "resumePausedUpload");
    await expect.poll(async () => (await hostResources(page)).paused_upload_operations).toBe(1);
    await expect(progress).toHaveAttribute("data-live-upload-percent", "66");
    await expect(progress).toHaveAttribute("aria-valuenow", "0");

    await page.evaluate(() => {
      const probe = Reflect.get(window, "__suprnovaIteration004") as {
        advanceUploadClock(milliseconds: number): void;
      };
      probe.advanceUploadClock(1);
    });
    await probeCommand(page, "resumePausedUpload");
    await expect(progress).toHaveAttribute("data-live-upload-state", "ready");
    await expect(progress).toHaveAttribute("data-live-upload-percent", "100");
    await expect(progress).toHaveAttribute("aria-valuenow", "100");
    await expect(progress).toHaveAttribute("aria-valuetext", "Upload ready at 100%");
    await page.getByRole("button", { name: "Remove upload" }).click();
    await expect.poll(async () => (await hostResources(page)).active_uploads).toBe(0);
  });
}

test("upload failure is visible associated and preserves native-input focus", async ({
  page,
}, testInfo) => {
  test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
  await page.goto(
    `${REFERENCE_ORIGIN}/scenario/iteration004?features=uploads&format=esm&upload-chunk-bytes=262145`,
  );
  await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
  const input = page.getByLabel("Iteration 004 file");
  await input.focus();
  await input.setInputFiles({
    buffer: Buffer.alloc(256 * 1024 + 1, 0x62),
    mimeType: "application/octet-stream",
    name: "rejected-chunk.bin",
  });
  const progress = page.locator("#iteration-upload-progress");
  await expect(progress).toHaveAttribute("data-live-upload-state", "failed");
  await expect(progress).toHaveAttribute("aria-invalid", "true");
  await expect(progress).toHaveAttribute("aria-errormessage", "iteration-upload-error");
  await expect(page.locator("#iteration-upload-error")).toBeVisible();
  await expect(page.locator("#iteration-upload-error")).toContainText("Upload failed");
  await expect(input).toBeFocused();
  await expect(page.getByRole("button", { name: "Retry upload" })).toBeEnabled();
  await page.getByRole("button", { name: "Remove upload" }).click();
  await expect.poll(async () => (await hostResources(page)).active_uploads).toBe(0);
});

for (const artifact of ["missing", "incompatible"] as const) {
  for (const format of ["esm", "classic"] as const) {
    for (const affected of ["upload", "async"] as const) {
      test(`${format} ${affected} ${artifact} artifact preserves native controls and focus`, async ({
        page,
      }, testInfo) => {
        test.skip(testInfo.project.name === "chrome-bfcache", "Lifecycle has a dedicated matrix.");
        await page.goto(
          `${REFERENCE_ORIGIN}/scenario/iteration004?features=both&format=${format}&${affected}-artifact=${artifact}`,
        );
        await expect(page.locator("html")).toHaveAttribute("data-iteration-004-ready", "true");
        await expect(
          page.getByRole("heading", { name: "Iteration 004 integration" }),
        ).toBeVisible();
        await page.getByText("Native disclosure", { exact: true }).focus();
        await page.keyboard.press("Enter");
        await expect(page.getByText("Native fallback details")).toBeVisible();
        await expect(page.getByText("Native disclosure", { exact: true })).toBeFocused();
        await expectNoSeriousA11yViolations(page, {
          sourceUrl: `${REFERENCE_ORIGIN}/scenario/iteration004-axe.js`,
        });
      });
    }
  }
}
