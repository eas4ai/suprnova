import { expect, test } from "@playwright/test";

import {
  dispatchPersistedLifecycle,
  installResourceInstrumentation,
  resourceSnapshot,
} from "./support/faults.js";
import { browserTaskBarrier } from "./support/event-loop-barrier.js";

test("repeated suspend and restore cycles return observed resources to baseline", async ({
  page,
}) => {
  const pageErrors: string[] = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  await installResourceInstrumentation(page);
  await page.goto("/scenario/lifecycle");
  expect(pageErrors).toEqual([]);
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
  const baseline = await resourceSnapshot(page);

  for (let cycle = 0; cycle < 20; cycle += 1) {
    await dispatchPersistedLifecycle(page, "pagehide");
    await dispatchPersistedLifecycle(page, "pageshow");
    const lifecycle = await page.evaluate(() => {
      const probe = (
        window as unknown as {
          readonly __suprnovaLifecycleProbe?: {
            readonly runtime: { status(): unknown };
          };
        }
      ).__suprnovaLifecycleProbe;
      return {
        island: document
          .querySelector('[data-suprnova-live-document-key="primary"]')
          ?.getAttribute("data-suprnova-live-status"),
        runtime: probe === undefined ? null : String(probe.runtime.status()),
      };
    });
    expect(lifecycle, `lifecycle cycle ${String(cycle + 1)}`).toEqual({
      island: "connected",
      runtime: "running",
    });
  }

  expect(await resourceSnapshot(page)).toEqual(baseline);
  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-status",
    "connected",
  );
});

test("repeated accepted morphs return observed resources to baseline", async ({ page }) => {
  await installResourceInstrumentation(page);
  await page.goto("/scenario/morphIdentity");
  const island = page.locator('[data-suprnova-live-document-key="primary"]');
  await expect(island).toHaveAttribute("data-suprnova-live-status", "connected");
  const baseline = await resourceSnapshot(page);

  for (let cycle = 0; cycle < 10; cycle += 1) {
    await page.locator("#morph-action").click();
    await expect(island).toHaveAttribute("data-suprnova-live-revision", String(8 + cycle));
    await expect.poll(async () => resourceSnapshot(page)).toEqual(baseline);
  }
});

test("connect, replace, and remove leave no retired callback authority", async ({ page }) => {
  await installResourceInstrumentation(page);
  let liveRequests = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests += 1;
  });
  await page.goto("/scenario/dynamic");
  const baseline = await resourceSnapshot(page);
  await page.evaluate(async () => {
    const template = document.querySelector<HTMLTemplateElement>("#candidate");
    if (template === null) throw new Error("dynamic_template_missing");
    const first = template.content.firstElementChild?.cloneNode(true);
    const replacement = template.content.firstElementChild?.cloneNode(true);
    if (!(first instanceof Element) || !(replacement instanceof Element)) {
      throw new Error("dynamic_candidate_missing");
    }
    const main = document.querySelector("main");
    if (main === null) throw new Error("dynamic_host_missing");
    const waitForStatus = (candidate: Element, expected: string | null): Promise<void> =>
      new Promise((resolve) => {
        if (candidate.getAttribute("data-suprnova-live-status") === expected) {
          resolve();
          return;
        }
        const observer = new MutationObserver(() => {
          if (candidate.getAttribute("data-suprnova-live-status") !== expected) return;
          observer.disconnect();
          resolve();
        });
        observer.observe(candidate, {
          attributeFilter: ["data-suprnova-live-status"],
          attributes: true,
        });
      });

    main.append(first);
    await waitForStatus(first, "connected");
    first.replaceWith(replacement);
    await waitForStatus(replacement, "connected");
    replacement.remove();
    await waitForStatus(replacement, "disconnected");

    for (const candidate of [first, replacement]) {
      const button = candidate.querySelector("button");
      if (!(button instanceof HTMLButtonElement)) throw new Error("dynamic_button_missing");
      button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    }
  });
  await browserTaskBarrier(page);

  expect(liveRequests).toBe(0);
  expect(await resourceSnapshot(page)).toEqual(baseline);
});
