import { expect, test } from "@playwright/test";

async function dispatchPersistedTransition(
  page: import("@playwright/test").Page,
  type: "pagehide" | "pageshow",
): Promise<void> {
  await page.evaluate((eventType) => {
    const event = new Event(eventType);
    Object.defineProperty(event, "persisted", { value: true });
    window.dispatchEvent(event);
  }, type);
}

test("restoration and duplicate artifact execution do not multiply lifecycle listeners", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const additions: Record<string, number> = {};
    const removals: Record<string, number> = {};
    const add: unknown = Reflect.get(EventTarget.prototype, "addEventListener");
    const remove: unknown = Reflect.get(EventTarget.prototype, "removeEventListener");
    if (typeof add !== "function" || typeof remove !== "function") {
      throw new Error("event_target_instrumentation_unavailable");
    }
    EventTarget.prototype.addEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      if (["freeze", "pagehide", "pageshow", "resume", "unload"].includes(type)) {
        additions[type] = (additions[type] ?? 0) + 1;
      }
      Reflect.apply(add, this, [type, listener, options]);
    };
    EventTarget.prototype.removeEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      if (["freeze", "pagehide", "pageshow", "resume", "unload"].includes(type)) {
        removals[type] = (removals[type] ?? 0) + 1;
      }
      Reflect.apply(remove, this, [type, listener, options]);
    };
    Reflect.set(window, "__lifecycleListeners", { additions, removals });
  });
  await page.goto("/scenario/lifecycle");
  const baseline = await page.evaluate(() => {
    const value: unknown = Reflect.get(window, "__lifecycleListeners");
    if (typeof value !== "object" || value === null) return {};
    const additions: unknown = Reflect.get(value, "additions");
    if (typeof additions !== "object" || additions === null) return {};
    const counts: Record<string, number> = {};
    for (const type of ["freeze", "pagehide", "pageshow", "resume", "unload"]) {
      const count: unknown = Reflect.get(additions, type);
      if (typeof count === "number") counts[type] = count;
    }
    return counts;
  });

  await page.evaluate(() => {
    const hide = new Event("pagehide");
    Object.defineProperty(hide, "persisted", { value: true });
    window.dispatchEvent(hide);
    const show = new Event("pageshow");
    Object.defineProperty(show, "persisted", { value: true });
    window.dispatchEvent(show);
    const probe: unknown = Reflect.get(window, "__suprnovaLifecycleProbe");
    if (typeof probe !== "object" || probe === null) return;
    const bootAgain: unknown = Reflect.get(probe, "bootAgain");
    if (typeof bootAgain === "function") Reflect.apply(bootAgain, probe, []);
  });
  const after = await page.evaluate(() => {
    const value: unknown = Reflect.get(window, "__lifecycleListeners");
    if (typeof value !== "object" || value === null) return {};
    const additions: unknown = Reflect.get(value, "additions");
    if (typeof additions !== "object" || additions === null) return {};
    const counts: Record<string, number> = {};
    for (const type of ["freeze", "pagehide", "pageshow", "resume", "unload"]) {
      const count: unknown = Reflect.get(additions, type);
      if (typeof count === "number") counts[type] = count;
    }
    return counts;
  });

  expect(after).toEqual(baseline);
  expect(after).not.toHaveProperty("unload");
});

test("an old-epoch transport response cannot mutate the restored document", async ({ page }) => {
  await page.goto("/scenario/lifecycle");
  await page.locator("#lifecycle-action").click();
  await page.waitForTimeout(30);
  await page.evaluate(() => {
    const hide = new Event("pagehide");
    Object.defineProperty(hide, "persisted", { value: true });
    window.dispatchEvent(hide);
    const show = new Event("pageshow");
    Object.defineProperty(show, "persisted", { value: true });
    window.dispatchEvent(show);
  });
  await page.waitForTimeout(400);

  await expect(page.locator("#lifecycle-content")).toHaveText("Lifecycle original");
  await expect(page.locator("[data-suprnova-live-island]")).toHaveAttribute(
    "data-suprnova-live-revision",
    "7",
  );
});

test("beforeunload exists only while an explicit dirty-work guard is active", async ({ page }) => {
  await page.addInitScript(() => {
    let active = 0;
    const add: unknown = Reflect.get(EventTarget.prototype, "addEventListener");
    const remove: unknown = Reflect.get(EventTarget.prototype, "removeEventListener");
    if (typeof add !== "function" || typeof remove !== "function") {
      throw new Error("event_target_instrumentation_unavailable");
    }
    EventTarget.prototype.addEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      if (this === window && type === "beforeunload") active += 1;
      Reflect.apply(add, this, [type, listener, options]);
    };
    EventTarget.prototype.removeEventListener = function (
      this: EventTarget,
      type,
      listener,
      options,
    ): void {
      if (this === window && type === "beforeunload") active -= 1;
      Reflect.apply(remove, this, [type, listener, options]);
    };
    Reflect.set(window, "__beforeUnloadCount", () => active);
  });
  await page.goto("/scenario/navigation");
  const count = () =>
    page.evaluate(() => {
      const read: unknown = Reflect.get(window, "__beforeUnloadCount");
      return typeof read === "function" ? Number(Reflect.apply(read, window, [])) : -1;
    });

  expect(await count()).toBe(0);
  await page.locator("#dirty-input").fill("unsaved");
  await expect.poll(count).toBe(1);
  await page.locator("#dirty-scope").evaluate((element) => {
    element.removeAttribute("data-suprnova-live-dirty");
  });
  await expect.poll(count).toBe(0);
});

test("a model debounce scheduled before persisted hide cannot fire while suspended", async ({
  page,
}) => {
  let liveRequests = 0;
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests += 1;
  });
  await page.goto("/scenario/modelsDebounce");
  await page.locator("#debounced-model").fill("suspended query");
  await dispatchPersistedTransition(page, "pagehide");
  await page.waitForTimeout(250);

  expect(liveRequests).toBe(0);
  await dispatchPersistedTransition(page, "pageshow");
  await page.waitForTimeout(150);
  expect(liveRequests).toBe(0);
});

test("an old-epoch transition is rejected before one fresh render recovers the island", async ({
  page,
}) => {
  const operationKinds: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname !== "/live") return;
    const body = request.postData();
    if (body === null) return;
    const payload: unknown = JSON.parse(body);
    if (typeof payload !== "object" || payload === null) return;
    const operations: unknown = Reflect.get(payload, "operations");
    if (!Array.isArray(operations)) return;
    for (const operation of operations as unknown[]) {
      if (typeof operation !== "object" || operation === null) continue;
      const kind: unknown = Reflect.get(operation, "kind");
      if (typeof kind === "string") operationKinds.push(kind);
    }
  });
  await page.goto("/scenario/transitions");
  const island = page.locator('[data-suprnova-live-document-key="primary"]');
  await page.locator("#transition-action").click();
  await expect(page.locator("#transition-leave")).toHaveAttribute(
    "data-suprnova-live-transition-state",
    "leave:fade",
  );

  await dispatchPersistedTransition(page, "pagehide");
  await dispatchPersistedTransition(page, "pageshow");
  await page.waitForTimeout(250);

  expect(operationKinds).toEqual(["invoke_action", "fresh_render"]);
  await expect(island).toHaveAttribute("data-suprnova-live-revision", "8");
  await expect(page.locator("#transition-state")).toHaveText("After");
  await expect(page.locator("#transition-leave")).toHaveCount(0);
});
