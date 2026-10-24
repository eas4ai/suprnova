import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("Live keys move existing identity while rekeys replace and nested islands stay opaque", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("morphIdentity");
  await page.evaluate(() => {
    const state = {
      alpha: document.querySelector("#alpha"),
      child: document.querySelector('[data-suprnova-live-document-key="morph-child"]'),
      old: document.querySelector("#old"),
    };
    Reflect.set(window, "__suprnovaMorphIdentity", state);
  });

  await page.locator("#morph-action").click();
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );

  await expect(page.locator("#morph-list > li")).toHaveText([
    "Beta updated",
    "Alpha updated",
    "New",
  ]);
  await expect(page.locator('[data-suprnova-live-document-key="morph-child"]')).toContainText(
    "Nested original",
  );
  expect(
    await page.evaluate(() => {
      const state = Reflect.get(window, "__suprnovaMorphIdentity") as {
        readonly alpha: Element | null;
        readonly child: Element | null;
        readonly old: Element | null;
      };
      return {
        alphaPreserved: state.alpha === document.querySelector("#alpha"),
        childPreserved:
          state.child === document.querySelector('[data-suprnova-live-document-key="morph-child"]'),
        oldDisconnected: state.old?.isConnected === false,
        rekeyCreated: state.old !== document.querySelector("#new"),
      };
    }),
  ).toEqual({
    alphaPreserved: true,
    childPreserved: true,
    oldDisconnected: true,
    rekeyCreated: true,
  });
});

test("prohibited response markup never mutates DOM or commits before fresh-render recovery", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("morphUnsafe");

  await page.locator("#morph-unsafe-action").click();
  await expect(page.locator("html")).toHaveAttribute("data-morph-recovery", "/morph-recovered");
  await expect(page.locator("#morph-unsafe-content")).toHaveText("Original");
  await expect(runtime.island()).toHaveAttribute("data-suprnova-live-revision", "7");
  await expect(page.locator("html")).not.toHaveAttribute("data-morph-script-executed", "true");
});
