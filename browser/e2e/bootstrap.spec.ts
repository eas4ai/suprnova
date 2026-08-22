import { expect, test } from "@playwright/test";

import { ISLAND_SELECTOR, RuntimePage, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

test("SSR content is visible before startup and a valid instanced island connects", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("manual");
  await runtime.expectVisibleContent("Server-rendered search results");
  await expect(runtime.island()).not.toHaveAttribute(STATUS_ATTRIBUTE, /.+/u);

  await page.addScriptTag({
    content: 'import { boot } from "/assets/suprnova-live.esm.js"; boot();',
    type: "module",
  });
  await runtime.expectStatus("connected");
});

test("a public seed connects without an eager endpoint request", async ({ page }) => {
  const liveRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
  });
  const runtime = new RuntimePage(page);
  await runtime.open("seed");
  await runtime.expectStatus("connected");
  expect(liveRequests).toEqual([]);
});

test("malformed and incompatible roots stay visible but disconnected", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("malformed");
  await runtime.expectVisibleContent("Malformed but visible");
  await runtime.expectStatus("invalid");

  await runtime.open("incompatible");
  await runtime.expectVisibleContent("Incompatible but visible");
  await runtime.expectStatus("incompatible");

  await runtime.open("snapshotMismatch");
  await runtime.expectVisibleContent("Mismatched but visible");
  await runtime.expectStatus("incompatible");
});

test("document-local duplicate roots connect once in deterministic order", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("duplicate");
  await runtime.expectStatus("connected", 0);
  await runtime.expectStatus("invalid", 1);
});

test("nested valid roots connect as independent records", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("nested");
  await runtime.expectStatus("connected", 0);
  await runtime.expectStatus("connected", 1);
});

test("runtime disposal is idempotent and retires the connected record", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("instance");
  await runtime.expectStatus("connected");
  const status = await page.evaluate(() => {
    const handle: unknown = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1"));
    if (typeof handle !== "object" || handle === null) throw new Error("runtime_missing");
    const stop: unknown = Reflect.get(handle, "stop");
    const readStatus: unknown = Reflect.get(handle, "status");
    if (typeof stop !== "function" || typeof readStatus !== "function") {
      throw new Error("runtime_handle_invalid");
    }
    Reflect.apply(stop, handle, []);
    Reflect.apply(stop, handle, []);
    return String(Reflect.apply(readStatus, handle, []));
  });
  expect(status).toBe("stopped");
  await expect(runtime.island()).toHaveAttribute(STATUS_ATTRIBUTE, "disconnected");
});

test("classic and ESM loading share one runtime and one connection", async ({ page }) => {
  await page.addInitScript(
    ({ selector, attribute }) => {
      const changes: string[] = [];
      Reflect.set(window, "__suprnovaStatusChanges", changes);
      new MutationObserver((records) => {
        for (const record of records) {
          if (
            record.type === "attributes" &&
            record.attributeName === attribute &&
            record.target instanceof Element &&
            record.target.matches(selector)
          ) {
            changes.push(record.target.getAttribute(attribute) ?? "");
          }
        }
      }).observe(document, { attributes: true, subtree: true });
    },
    { selector: ISLAND_SELECTOR, attribute: STATUS_ATTRIBUTE },
  );
  const runtime = new RuntimePage(page);
  await runtime.open("duplicateRuntime");
  await runtime.expectStatus("connected");
  expect(
    await page.evaluate(() => {
      const value: unknown = Reflect.get(window, "__suprnovaStatusChanges");
      return Array.isArray(value) ? value.map((entry: unknown) => String(entry)) : [];
    }),
  ).toEqual(["connected"]);
});

test("dynamic insertion uses normal discovery and removal disposes exactly once", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("dynamic");
  const result = await page.evaluate(
    async ({ selector, attribute }) => {
      const template = document.querySelector<HTMLTemplateElement>("#candidate");
      if (template === null) throw new Error("candidate_missing");
      const candidate = template.content.firstElementChild?.cloneNode(true);
      if (!(candidate instanceof Element)) throw new Error("candidate_invalid");
      document.querySelector("main")?.append(candidate);
      for (
        let attempt = 0;
        attempt < 20 && candidate.getAttribute(attribute) !== "connected";
        attempt += 1
      ) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      const connected = candidate.getAttribute(attribute);
      candidate.remove();
      await new Promise((resolve) => setTimeout(resolve, 20));
      return {
        connected,
        disposed: candidate.getAttribute(attribute),
        survivors: document.querySelectorAll(selector).length,
      };
    },
    { selector: ISLAND_SELECTOR, attribute: STATUS_ATTRIBUTE },
  );
  expect(result).toEqual({ connected: "connected", disposed: "disconnected", survivors: 0 });
});
