import { expect, test } from "@playwright/test";

import { RuntimePage } from "./support/runtime-page.js";

function liveRequests(page: import("@playwright/test").Page) {
  const requests: import("@playwright/test").Request[] = [];
  page.on("request", (request) => {
    if (new URL(request.url()).pathname === "/live") requests.push(request);
  });
  return requests;
}

test("instanced actions send the canonical bounded Live envelope", async ({ page }) => {
  const requests = liveRequests(page);
  const runtime = new RuntimePage(page);
  await runtime.open("networkInstance");
  await runtime.expectStatus("connected");
  await page.locator("#network-action").click();
  await expect.poll(() => requests.length).toBe(1);

  const request = requests[0];
  if (request === undefined) throw new Error("missing live request");
  const body = request.postDataJSON() as Record<string, unknown>;
  expect(request.method()).toBe("POST");
  expect(request.headers()["content-type"]).toBe(
    "application/vnd.suprnova.live+json; charset=utf-8; version=1",
  );
  expect(body["base_revision"]).toBe("7");
  expect(body["snapshot"]).toMatchObject({ kind: "instance" });
  expect(body["extensions"]).toEqual({ x_suprnova_live_document_key_v1: "primary" });
  expect(body["correlation_id"]).toMatch(/^[A-Za-z0-9_-]{22}$/u);
  expect(body["idempotency_key"]).toMatch(/^[A-Za-z0-9_-]{22}$/u);
});

test("safe retry preserves exact bytes and immutable identity", async ({ page }) => {
  const requests = liveRequests(page);
  const runtime = new RuntimePage(page);
  await runtime.open("networkRetry");
  await page.locator("#network-action").click();
  await expect.poll(() => requests.length).toBe(2);

  expect(requests[0]?.postData()).toBe(requests[1]?.postData());
  expect(requests[0]?.headers()["accept"]).toBe(requests[1]?.headers()["accept"]);
});

test("seed promotion inserts one intent-owned nonce", async ({ page }) => {
  const requests = liveRequests(page);
  const runtime = new RuntimePage(page);
  await runtime.open("networkSeed");
  await page.locator("#network-action").click();
  await expect.poll(() => requests.length).toBe(1);

  const body = requests[0]?.postDataJSON() as Record<string, unknown>;
  expect(body["base_revision"]).toBe("0");
  expect(body["snapshot"]).toMatchObject({
    browser_nonce: expect.stringMatching(/^[A-Za-z0-9_-]{22}$/u),
    kind: "seed_promotion",
  });
});
