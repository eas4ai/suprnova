import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

test("delegated actions preserve native behavior and apply validated modifiers once", async ({
  page,
}) => {
  const liveRequests: string[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") liveRequests.push(request.url());
  });
  const runtime = new RuntimePage(page);
  await runtime.open("directives");
  await runtime.expectStatus("connected");

  const results = await page.evaluate(() => {
    const dispatch = (selector: string, event: Event) =>
      document.querySelector(selector)?.dispatchEvent(event);
    return {
      disabled: dispatch(
        "#disabled-action",
        new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
      ),
      enter: dispatch(
        "#key-action",
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          composed: true,
          key: "Enter",
        }),
      ),
      escape: dispatch(
        "#key-action",
        new KeyboardEvent("keydown", {
          bubbles: true,
          cancelable: true,
          composed: true,
          key: "Escape",
        }),
      ),
      first: dispatch(
        "#once-action",
        new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
      ),
      second: dispatch(
        "#once-action",
        new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
      ),
      syntheticTrusted: dispatch(
        "#trusted-action",
        new MouseEvent("click", { bubbles: true, cancelable: true, composed: true }),
      ),
    };
  });

  expect(results).toEqual({
    disabled: true,
    enter: false,
    escape: true,
    first: false,
    second: true,
    syntheticTrusted: true,
  });
  await page.locator("#native-action").click();
  await expect(page).toHaveURL(/#native$/u);
  await page.evaluate(() => {
    history.replaceState(null, "", location.pathname);
  });
  await page.locator("#trusted-action").click();
  await expect(page).not.toHaveURL(/#trusted$/u);

  const provenance = await page.evaluate(async () => {
    const event = () =>
      new MouseEvent("click", { bubbles: true, cancelable: true, composed: true });
    const late = document.querySelector("#late-action");
    late?.setAttribute("live:click.prevent", "late");
    const attributeOnly = late?.dispatchEvent(event());

    const inserted = document.createElement("button");
    inserted.setAttribute("live:click.prevent", "inserted");
    inserted.textContent = "Inserted action";
    document.querySelector("[data-suprnova-live-island]")?.append(inserted);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        resolve();
      }),
    );
    const insertedResult = inserted.dispatchEvent(event());

    late?.remove();
    if (late === null) throw new Error("missing_late_action");
    document.querySelector("[data-suprnova-live-island]")?.append(late);
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => {
        resolve();
      }),
    );
    const revalidated = late.dispatchEvent(event());

    const removed = document.querySelector("#remove-action");
    removed?.addEventListener(
      "click",
      () => {
        removed.remove();
      },
      { once: true },
    );
    const removedDuringDispatch = removed?.dispatchEvent(event());
    return { attributeOnly, insertedResult, removedDuringDispatch, revalidated };
  });
  expect(provenance).toEqual({
    attributeOnly: true,
    insertedResult: false,
    removedDuringDispatch: true,
    revalidated: false,
  });
  expect(liveRequests).toEqual([]);
});
