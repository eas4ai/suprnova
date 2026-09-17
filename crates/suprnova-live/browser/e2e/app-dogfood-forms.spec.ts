import { expect, test, type Page, type Request, type Route } from "@playwright/test";

import { ISLAND_SELECTOR, STATUS_ATTRIBUTE } from "./support/runtime-page.js";

// The dogfood form gallery through the real application: the controls show
// the island's state (FORM-009), a group selection the server has not yet
// answered survives a re-render (FORM-010), a checkbox group proposes a list
// (LIVE-032), and a reset replaces an edit the island refused. Every gallery
// control sends its update as the user edits, so a selection is unanswered
// only while an earlier request is on its way; that case holds one to open
// the window.
const APP_ORIGIN = "http://127.0.0.1:4178";
const ACTION_ROUTE = "**/__live/action";

async function openGallery(page: Page): Promise<void> {
  await page.goto(`${APP_ORIGIN}/live/demo-login`);
  await expect(page).toHaveURL(`${APP_ORIGIN}/live`);
  await page.goto(`${APP_ORIGIN}/live/forms`);
  const island = page.locator(ISLAND_SELECTOR);
  await expect(island).toHaveCount(1);
  await expect(island).toHaveAttribute(STATUS_ATTRIBUTE, "connected");
}

interface ActionBody {
  readonly model_proposals?: Readonly<Record<string, unknown>>;
}

function actionNamed(name: string): (request: Request) => boolean {
  return (request) =>
    request.url().includes("/__live/action") &&
    (request.postData() ?? "").includes(`"name":"${name}"`);
}

function proposals(request: Request): Readonly<Record<string, unknown>> {
  const body = request.postDataJSON() as ActionBody;
  return body.model_proposals ?? {};
}

async function fillRequired(page: Page): Promise<void> {
  await page.locator("#email").fill("ada@example.com");
  await page.locator("#secret").fill("correct horse battery staple");
  await page.locator("#agree").check();
}

interface Hold {
  /** The next held request, in the order the page sent them. */
  next(): Promise<Route>;
  /** Sends every held request and stops holding. */
  release(): Promise<void>;
}

/** Holds each Live request whose body `matches` until the case lets it go. */
async function holdActions(page: Page, matches: (body: string) => boolean): Promise<Hold> {
  const held: Route[] = [];
  const waiting: ((route: Route) => void)[] = [];
  let holding = true;
  await page.route(ACTION_ROUTE, async (route) => {
    if (!holding || !matches(route.request().postData() ?? "")) {
      await route.continue();
      return;
    }
    const waiter = waiting.shift();
    if (waiter === undefined) held.push(route);
    else waiter(route);
  });
  return {
    next: () =>
      new Promise<Route>((resolve) => {
        const route = held.shift();
        if (route === undefined) waiting.push(resolve);
        else resolve(route);
      }),
    release: async () => {
      holding = false;
      for (const route of held.splice(0)) await route.continue();
    },
  };
}

const syncs =
  (...fields: readonly string[]) =>
  (body: string) =>
    fields.some((field) => body.includes(`"field":"${field}","kind":"sync_model"`));

test("FORM-009: the gallery's controls show the island's values and an untouched save sends them back", async ({
  page,
}) => {
  await openGallery(page);
  await expect(page.locator("#quantity")).toHaveValue("1");
  await expect(page.locator("#volume")).toHaveValue("50");
  await expect(page.locator("#bio")).toHaveValue("");

  await fillRequired(page);
  const request = page.waitForRequest(actionNamed("save"));
  const response = page.waitForResponse((reply) => actionNamed("save")(reply.request()));
  await page.getByRole("button", { name: "Save" }).click();
  const sent = proposals(await request);
  expect(sent["quantity"]).toBe(1);
  expect(sent["volume"]).toBe(50);
  expect(sent["topics"]).toEqual([]);
  expect(sent["newsletter"]).toBe(false);
  expect((await response).status()).toBe(200);
  await expect(page.locator("#quantity")).toHaveValue("1");
  await expect(page.locator("#email")).toHaveValue("ada@example.com");
});

test("FORM-010 and LIVE-032: a group selection the server has not answered survives a re-render and saves as a list", async ({
  page,
}) => {
  await openGallery(page);
  const releases = page.locator('input[name="topics"][value="releases"]');
  const security = page.locator('input[name="topics"][value="security"]');
  const team = page.locator('input[name="plan"][value="team"]');

  // The search update is on its way, so the selection's own updates wait
  // behind it and are held as they leave; the search's re-render is the
  // first to arrive and renders neither choice.
  const hold = await holdActions(page, syncs("query", "topics", "plan"));
  await page.locator("#query").fill("rust");
  const query = await hold.next();
  await releases.check();
  await team.check();
  await query.continue();
  await expect(page.locator("#query")).toHaveAttribute("value", "rust");
  await expect(releases).toBeChecked();
  await expect(team).toBeChecked();

  // A second box read after that re-render joins the first, not replaces it.
  await security.check();
  await hold.release();
  await fillRequired(page);
  const request = page.waitForRequest(actionNamed("save"));
  await page.getByRole("button", { name: "Save" }).click();
  const sent = proposals(await request);
  expect(sent["topics"]).toEqual(["releases", "security"]);
  expect(sent["plan"]).toBe("team");
  await expect(releases).toBeChecked();
  await expect(security).toBeChecked();
});

test("FORM-009: a reset replaces an edit the island refused, and only that render does", async ({
  page,
}) => {
  await openGallery(page);
  const quantity = page.locator("#quantity");
  const answered = (field: string) =>
    page.waitForResponse(
      (reply) =>
        reply.url().includes("/__live/action") && syncs(field)(reply.request().postData() ?? ""),
    );

  // An empty seat count is a value the island's u64 field cannot take: the
  // island keeps 1, and the control keeps what the user typed beside the
  // error, so the render answering the reset holds the value the previous
  // render held.
  let refused = answered("quantity");
  await quantity.fill("");
  await refused;
  await expect(quantity).toHaveValue("");
  const reset = page.waitForResponse((reply) => actionNamed("reset")(reply.request()));
  await page.getByRole("button", { name: "Reset" }).click();
  expect((await reset).status()).toBe(200);
  await expect(quantity).toHaveValue("1");

  // A refused edit made after the reset survives the next re-render. The
  // valid edit before it is what makes the empty count a new proposal.
  const accepted = answered("quantity");
  await quantity.fill("5");
  await accepted;
  refused = answered("quantity");
  await quantity.fill("");
  await refused;
  const search = answered("query");
  await page.locator("#query").fill("docs");
  expect((await search).status()).toBe(200);
  await expect(page.locator("#query")).toHaveAttribute("value", "docs");
  await expect(quantity).toHaveValue("");
});
