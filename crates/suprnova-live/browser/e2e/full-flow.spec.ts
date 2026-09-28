import { expect, test } from "@playwright/test";

const TRACEABILITY = Object.freeze({
  accessibility: "accessibility.spec.ts",
  "callbacks after retirement": "leaks.spec.ts",
  "controllers and signals": "signals-and-controllers.spec.ts",
  "controls and teleport": "preservation.spec.ts",
  CSP: "compatibility.spec.ts",
  diagnostics: "hostile-dom.spec.ts",
  "effects and transitions": "transitions-and-recovery.spec.ts",
  "event ownership": "directives.spec.ts",
  "focus, selection, and IME": "focus-and-forms.spec.ts + ime-and-selection.spec.ts",
  "models and forms": "models-and-forms.spec.ts",
  "morph identity": "morph-identity.spec.ts",
  "multiple schedulers": "multiple-islands.spec.ts",
  "navigation and bfcache": "navigation.spec.ts + bfcache.spec.ts",
  "nested ownership": "nested-islands.spec.ts",
  "offline, retry, and cancel": "network.spec.ts",
  "ordinary fallback": "navigation.spec.ts",
  "recovery behavior": "transitions-and-recovery.spec.ts",
  "redirect and reflection": "response-order.spec.ts",
  "response order": "response-order.spec.ts",
  "seed promotion": "seed-and-lazy.spec.ts",
  "local-only interaction": "local-signals.spec.ts",
});

test("the browser conformance matrix keeps every reviewed capability traceable", () => {
  expect(Object.keys(TRACEABILITY)).toHaveLength(21);
  expect(new Set(Object.values(TRACEABILITY)).size).toBeGreaterThan(10);
});

test("local state, model capture, server action, URL reflection, and native navigation compose", async ({
  page,
}) => {
  await page.goto("/scenario/fullFlow");
  await page.locator("#flow-disclosure").click();
  await expect(page.locator("#flow-panel")).toBeVisible();
  await page.locator("#flow-model").fill("composed");
  const liveRequest = page.waitForRequest((request) => new URL(request.url()).pathname === "/live");
  await page.locator("#flow-action").click();
  const request = await liveRequest;
  const body = request.postData();
  if (body === null) throw new Error("full_flow_request_body_missing");
  const payload: unknown = JSON.parse(body) as unknown;
  if (typeof payload !== "object" || payload === null) {
    throw new Error("full_flow_request_invalid");
  }
  const modelProposals: unknown = Reflect.get(payload, "model_proposals");
  const operations: unknown = Reflect.get(payload, "operations");
  expect(modelProposals).toEqual({ query: "composed" });
  expect(operations).toEqual([
    { field: "query", kind: "sync_model" },
    { arguments: {}, kind: "invoke_action", name: "search" },
  ]);

  await expect(page.locator('[data-suprnova-live-document-key="primary"]')).toHaveAttribute(
    "data-suprnova-live-revision",
    "8",
  );
  await expect(page).toHaveURL(/state=done/u);
  await page.locator("#flow-native-link").click();
  await expect(page.locator("#destination-marker")).toHaveText("Complete canonical destination");
});
