import { expect, test } from "@playwright/test";

import { expectNoSeriousA11yViolations } from "./support/a11y.js";

test("native uploads keep accessible progress across a compatible morph and retire on rekey", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/scenario/uploads");
  await expect(page.locator("html")).toHaveAttribute("data-upload-runtime", "ready");

  const input = page.locator("#attachment-input");
  const progress = page.locator("#attachment-progress");
  await input.setInputFiles({
    buffer: Buffer.from("suprnova-live-upload"),
    mimeType: "text/plain",
    name: "report.txt",
  });
  await page.waitForFunction(
    () => typeof Reflect.get(window, "__releaseUploadChunk") === "function",
  );

  expect(
    await page.evaluate(() => {
      const selected = document.querySelector("#attachment-input");
      const island = document.querySelector("[data-suprnova-live-island]");
      const registration: unknown = Reflect.get(window, "__uploadRegistration") as unknown;
      const trusted: unknown = Reflect.get(window, "__uploadChangeTrusted") as unknown;
      return {
        files: selected instanceof HTMLInputElement ? selected.files?.length : null,
        registration: typeof registration === "string" ? registration : null,
        release: typeof Reflect.get(window, "__releaseUploadChunk"),
        status: island?.getAttribute("data-suprnova-live-status"),
        trusted: typeof trusted === "boolean" ? trusted : null,
      };
    }),
  ).toEqual({
    files: 1,
    registration: "registered",
    release: "function",
    status: "connected",
    trusted: false,
  });

  await expect(progress).toHaveAttribute("data-live-upload-state", "transferring");
  await expect(progress).toHaveAttribute("aria-busy", "true");
  await expect(progress).toHaveAttribute("data-live-upload-motion", "reduced");
  await expect(page.locator("#attachment-retry")).toBeDisabled();
  await expect(page.locator("#attachment-cancel")).toBeEnabled();
  await expectNoSeriousA11yViolations(page, { sourceUrl: "/test-vendor/axe.js" });

  await page.evaluate(() => {
    const release: unknown = Reflect.get(window, "__releaseUploadChunk");
    if (typeof release !== "function") throw new Error("upload_release_missing");
    Reflect.apply(release, window, []);
  });
  await expect(progress).toHaveAttribute("data-live-upload-state", "ready");
  await expect(progress).toHaveAttribute("aria-valuenow", "100");
  await expect(progress).toHaveAttribute("aria-busy", "false");

  await page.locator("#attachment-morph").click();
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  expect(
    await page.evaluate(() => {
      const initial: unknown = Reflect.get(window, "__uploadInitialInput");
      const valueWrites: unknown = Reflect.get(window, "__uploadValueWrites") as unknown;
      const filesWrites: unknown = Reflect.get(window, "__uploadFilesWrites") as unknown;
      const current = document.querySelector("#attachment-input");
      return {
        files: current instanceof HTMLInputElement ? current.files?.length : null,
        filesWriteCount: Array.isArray(filesWrites) ? filesWrites.length : null,
        same: initial === current,
        valueWriteCount: Array.isArray(valueWrites) ? valueWrites.length : null,
      };
    }),
  ).toEqual({ files: 1, filesWriteCount: 0, same: true, valueWriteCount: 0 });

  await page.locator("#attachment-morph").click();
  await page.waitForFunction(() => {
    const island = document.querySelector("[data-suprnova-live-island]");
    return (
      island?.getAttribute("data-suprnova-live-revision") === "9" ||
      island?.getAttribute("data-suprnova-live-status") === "disconnected"
    );
  });
  expect(
    await page.evaluate(() => {
      const island = document.querySelector("[data-suprnova-live-island]");
      const initial: unknown = Reflect.get(window, "__uploadInitialInput") as unknown;
      const valueWrites: unknown = Reflect.get(window, "__uploadValueWrites") as unknown;
      const filesWrites: unknown = Reflect.get(window, "__uploadFilesWrites") as unknown;
      if (!(initial instanceof HTMLInputElement)) throw new Error("upload_initial_input_missing");
      const firstValueWrite: unknown =
        Array.isArray(valueWrites) && valueWrites.length > 0
          ? (Reflect.get(valueWrites, 0) as unknown)
          : null;
      const current = document.querySelector("#attachment-input-replacement");
      return {
        filesWriteCount: Array.isArray(filesWrites) ? filesWrites.length : null,
        initialFiles: initial.files?.length,
        progress: document
          .querySelector("#attachment-progress-replacement")
          ?.getAttribute("data-live-upload-state"),
        replacement: current !== null,
        revision: island?.getAttribute("data-suprnova-live-revision"),
        same: initial === current,
        status: island?.getAttribute("data-suprnova-live-status"),
        valueWriteCount: Array.isArray(valueWrites) ? valueWrites.length : null,
        valueWriteFirst: typeof firstValueWrite === "string" ? firstValueWrite : null,
      };
    }),
  ).toEqual({
    filesWriteCount: 0,
    initialFiles: 0,
    progress: "canceled",
    replacement: true,
    revision: "9",
    same: false,
    status: "connected",
    valueWriteCount: 1,
    valueWriteFirst: "",
  });
  await expect(page.locator("#attachment-progress-replacement")).toHaveAttribute(
    "data-live-upload-state",
    "canceled",
  );
  expect(
    await page.evaluate(() => {
      const initial: unknown = Reflect.get(window, "__uploadInitialInput") as unknown;
      const valueWrites: unknown = Reflect.get(window, "__uploadValueWrites") as unknown;
      const filesWrites: unknown = Reflect.get(window, "__uploadFilesWrites") as unknown;
      if (!(initial instanceof HTMLInputElement)) throw new Error("upload_initial_input_missing");
      const firstValueWrite: unknown =
        Array.isArray(valueWrites) && valueWrites.length > 0
          ? (Reflect.get(valueWrites, 0) as unknown)
          : null;
      const current = document.querySelector("#attachment-input-replacement");
      return {
        bodyHasPath: document.body.textContent.includes("C:\\fakepath\\"),
        currentFiles: current instanceof HTMLInputElement ? current.files?.length : null,
        filesWriteCount: Array.isArray(filesWrites) ? filesWrites.length : null,
        initialFiles: initial.files?.length,
        same: initial === current,
        valueWriteCount: Array.isArray(valueWrites) ? valueWrites.length : null,
        valueWriteFirst: typeof firstValueWrite === "string" ? firstValueWrite : null,
      };
    }),
  ).toEqual({
    bodyHasPath: false,
    currentFiles: 0,
    filesWriteCount: 0,
    initialFiles: 0,
    same: false,
    valueWriteCount: 1,
    valueWriteFirst: "",
  });
  await expectNoSeriousA11yViolations(page, { sourceUrl: "/test-vendor/axe.js" });

  expect(
    await page.evaluate(() => {
      const violations: unknown = Reflect.get(window, "__uploadCspViolations") as unknown;
      return Array.isArray(violations) ? violations.length : null;
    }),
  ).toBe(0);
});
