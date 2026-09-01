import { expect, test, type Page } from "@playwright/test";

import { ISLAND_SELECTOR, RuntimePage, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

async function installStoppingFeatureDriver(page: Page, event: 0 | 1 | null) {
  await page.evaluate((stopEvent) => {
    const events: number[] = [];
    const islandPorts: unknown[] = [];
    Reflect.set(window, "__featureDriverEvents", events);
    Reflect.set(window, "__featureIslandPorts", islandPorts);
    const driver = Object.freeze([
      Symbol.for("suprnova.live.feature-driver.v1"),
      1,
      1_099_511_758_848,
      Object.freeze({}),
      (driverEvent: number, value: unknown) => {
        events.push(driverEvent);
        if (driverEvent === 1) islandPorts.push(value);
        if (driverEvent === stopEvent) {
          const runtime: unknown = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1"));
          if ((typeof runtime !== "object" && typeof runtime !== "function") || runtime === null) {
            throw new Error("runtime_missing");
          }
          const stop: unknown = Reflect.get(runtime, "stop");
          if (typeof stop !== "function") throw new Error("runtime_stop_missing");
          Reflect.apply(stop, runtime, []);
        }
        return true;
      },
    ]);
    const surface = { register: () => "registered", version: 1 };
    Object.defineProperty(surface, Symbol.for("suprnova.live.features.v1.adopt"), {
      value: () => driver,
    });
    Object.defineProperty(window, Symbol.for("suprnova.live.features.v1"), {
      value: Object.freeze(surface),
    });
  }, event);
}

async function waitForFeatureDriverStop(page: Page) {
  await page.waitForFunction(() => {
    const events: unknown = Reflect.get(window, "__featureDriverEvents");
    return Array.isArray(events) && events.length > 0 && events[events.length - 1] === 5;
  });
}

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

test("core-only boot stays operational when optional feature artifacts are absent", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("manual");
  await page.evaluate(
    ({ selector }) => {
      const island = document.querySelector(selector);
      if (!(island instanceof Element)) throw new Error("island_missing");
      island.setAttribute("live:poll.5s", "refresh");
      const input = document.createElement("input");
      input.setAttribute("live:upload", "avatar");
      island.append(input);
    },
    { selector: ISLAND_SELECTOR },
  );

  await page.addScriptTag({
    content: 'import { boot } from "/assets/suprnova-live.esm.js"; boot();',
    type: "module",
  });

  await runtime.expectStatus("connected");
  await expect(runtime.island()).toHaveAttribute("live:poll.5s", "refresh");
  await expect(runtime.island().locator('input[live\\:upload="avatar"]')).toHaveCount(1);
  const optionalRequests = await page.evaluate(() =>
    performance
      .getEntriesByType("resource")
      .map(({ name }) => name)
      .filter((name) => /suprnova-live\.(?:stimulus|uploads|async)\./u.test(name)),
  );
  expect(optionalRequests).toEqual([]);
});

test("a feature driver cannot resurrect startup after stopping core during event 0", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("manual");
  await installStoppingFeatureDriver(page, 0);
  await page.addScriptTag({
    content: 'import { boot } from "/assets/suprnova-live.esm.js"; boot();',
    type: "module",
  });
  await waitForFeatureDriverStop(page);

  expect(
    await page.evaluate(() => {
      const runtimeHandle: unknown = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1"));
      if (
        (typeof runtimeHandle !== "object" && typeof runtimeHandle !== "function") ||
        runtimeHandle === null
      ) {
        throw new Error("runtime_missing");
      }
      const status: unknown = Reflect.get(runtimeHandle, "status");
      if (typeof status !== "function") throw new Error("runtime_status_missing");
      const events: unknown = Reflect.get(window, "__featureDriverEvents");
      const runtimeStatus: unknown = Reflect.apply(status, runtimeHandle, []);
      return {
        events,
        status: runtimeStatus,
      };
    }),
  ).toEqual({ events: [0, 5], status: "stopped" });
});

test("a feature driver cannot continue island startup after stopping core during event 1", async ({
  page,
}) => {
  const runtime = new RuntimePage(page);
  await runtime.open("manual");
  await installStoppingFeatureDriver(page, 1);
  await page.addScriptTag({
    content: 'import { boot } from "/assets/suprnova-live.esm.js"; boot();',
    type: "module",
  });
  await waitForFeatureDriverStop(page);

  expect(
    await page.evaluate(() => {
      const runtimeHandle: unknown = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1"));
      if (
        (typeof runtimeHandle !== "object" && typeof runtimeHandle !== "function") ||
        runtimeHandle === null
      ) {
        throw new Error("runtime_missing");
      }
      const status: unknown = Reflect.get(runtimeHandle, "status");
      if (typeof status !== "function") throw new Error("runtime_status_missing");
      const events: unknown = Reflect.get(window, "__featureDriverEvents");
      const runtimeStatus: unknown = Reflect.apply(status, runtimeHandle, []);
      return {
        events,
        status: runtimeStatus,
      };
    }),
  ).toEqual({ events: [0, 1, 4, 5], status: "stopped" });
});

test("retained feature island ports stay inert after document disposal", async ({ page }) => {
  const runtime = new RuntimePage(page);
  await runtime.open("manual");
  await installStoppingFeatureDriver(page, null);
  await page.addScriptTag({
    content: 'import { boot } from "/assets/suprnova-live.esm.js"; boot();',
    type: "module",
  });
  await runtime.expectStatus("connected");

  expect(
    await page.evaluate(() => {
      const runtimeHandle: unknown = Reflect.get(window, Symbol.for("suprnova.live.runtime.v1"));
      if (
        (typeof runtimeHandle !== "object" && typeof runtimeHandle !== "function") ||
        runtimeHandle === null
      ) {
        throw new Error("runtime_missing");
      }
      const stop: unknown = Reflect.get(runtimeHandle, "stop");
      if (typeof stop !== "function") throw new Error("runtime_stop_missing");
      Reflect.apply(stop, runtimeHandle, []);
      const ports: unknown = Reflect.get(window, "__featureIslandPorts");
      const port: unknown = Array.isArray(ports) ? (ports as unknown[])[0] : undefined;
      if (typeof port !== "object" || port === null) throw new Error("feature_port_missing");
      const writePresentationSignal: unknown = Reflect.get(port, "writePresentationSignal");
      const enqueueFreshRender: unknown = Reflect.get(port, "enqueueFreshRender");
      if (
        typeof writePresentationSignal !== "function" ||
        typeof enqueueFreshRender !== "function"
      ) {
        throw new Error("feature_port_invalid");
      }
      let signalRejected = false;
      try {
        Reflect.apply(writePresentationSignal, port, [
          document.querySelector(ISLAND_SELECTOR),
          "open",
          true,
        ]);
      } catch {
        signalRejected = true;
      }
      const disposition: unknown = Reflect.apply(enqueueFreshRender, port, ["poll"]);
      const events: unknown = Reflect.get(window, "__featureDriverEvents");
      return {
        disposition,
        events,
        signalRejected,
      };
    }),
  ).toEqual({ disposition: "retired", events: [0, 1, 4, 5], signalRejected: true });
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
      const main = document.querySelector("main");
      if (main === null) throw new Error("main_missing");
      const observeStatus = (expected: string) => {
        let observer: MutationObserver | undefined;
        const reached = new Promise<void>((resolve) => {
          if (candidate.getAttribute(attribute) === expected) {
            resolve();
            return;
          }
          observer = new MutationObserver(() => {
            if (candidate.getAttribute(attribute) === expected) resolve();
          });
          observer.observe(candidate, { attributeFilter: [attribute], attributes: true });
        });
        return { disconnect: () => observer?.disconnect(), reached };
      };
      const connectedStatus = observeStatus("connected");
      try {
        main.append(candidate);
        await connectedStatus.reached;
        const connected = candidate.getAttribute(attribute);
        const disposedStatus = observeStatus("disconnected");
        try {
          candidate.remove();
          await disposedStatus.reached;
          return {
            connected,
            disposed: candidate.getAttribute(attribute),
            survivors: document.querySelectorAll(selector).length,
          };
        } finally {
          disposedStatus.disconnect();
        }
      } finally {
        connectedStatus.disconnect();
        candidate.remove();
      }
    },
    { selector: ISLAND_SELECTOR, attribute: STATUS_ATTRIBUTE },
  );
  expect(result).toEqual({ connected: "connected", disposed: "disconnected", survivors: 0 });
});
