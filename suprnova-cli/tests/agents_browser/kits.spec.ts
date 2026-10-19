import { expect, test, type Page } from '@playwright/test';

// AGENT-014 and AGENT-015 in a scaffolded Inertia kit, in Chromium with
// WebMCP enabled, observed as a built-in agent observes them: through the
// DevTools `WebMCP` domain and `document.modelContext`, each call made by
// `WebMCP.invokeTool`. `kits.rs` scaffolds the kit, adds the fixture pages
// and routes of `kit_fixtures/app`, starts the application and sets
// KIT_NAME and KIT_BASE_URL.

const kit = process.env.KIT_NAME ?? '';
const password = 'agents-kit-password-42';

test.skip(kit === '', 'Run through tests/agents_browser/kits.rs, which sets KIT_NAME.');

interface ToolResponse {
  readonly status: string;
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
    readonly content?: { text: string }[];
  };
}

interface Recorded {
  readonly method: string;
  readonly path: string;
  readonly tool: string | null;
  readonly inertia: boolean;
  readonly csrf: boolean;
  readonly cookie: boolean;
}

interface Descriptor {
  readonly name: string;
  readonly title?: string;
  readonly description: string;
  readonly inputSchema: Record<string, unknown>;
  readonly annotations: Record<string, boolean>;
}

// Every descriptor the page hands `registerTool`, recorded before the real
// call, so a test sees `consequentialHint`, which Chromium does not keep.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const recorded: unknown[] = [];
    Object.defineProperty(window, '__registered', { value: recorded });
    const context = (document as unknown as { modelContext?: Record<string, unknown> }).modelContext;
    if (context === undefined || typeof context.registerTool !== 'function') return;
    const register = (context.registerTool as (...args: unknown[]) => unknown).bind(context);
    context.registerTool = (tool: Record<string, unknown>, options: unknown) => {
      recorded.push(
        JSON.parse(
          JSON.stringify({
            name: tool.name,
            title: tool.title,
            description: tool.description,
            inputSchema: tool.inputSchema,
            annotations: tool.annotations,
          }),
        ),
      );
      return register(tool, options);
    };
  });
});

async function webmcp(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable' as never);
  const added: string[] = [];
  const removed: string[] = [];
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded' as never, (event: { tools: { name: string; frameId: string }[] }) => {
    for (const tool of event.tools) {
      added.push(tool.name);
      frameId = tool.frameId;
    }
  });
  cdp.on('WebMCP.toolsRemoved' as never, (event: { tools: { name: string }[] }) => {
    for (const tool of event.tools) removed.push(tool.name);
  });
  cdp.on('WebMCP.toolResponded' as never, (event: ToolResponse & { invocationId: string }) => {
    responses.set(event.invocationId, event);
  });
  return {
    added,
    removed,
    async invoke(toolName: string, input: Record<string, unknown>): Promise<ToolResponse> {
      const { invocationId } = (await cdp.send('WebMCP.invokeTool' as never, {
        frameId,
        toolName,
        input,
      } as never)) as { invocationId: string };
      await expect.poll(() => responses.has(invocationId)).toBe(true);
      return responses.get(invocationId) as ToolResponse;
    },
  };
}

/** The fixture's tools the browser holds for the page, sorted. */
async function registered(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools())
      .map((tool) => tool.name)
      .filter((name) => name.startsWith('fixture.'))
      .sort();
  });
}

async function descriptors(page: Page): Promise<Descriptor[]> {
  return page.evaluate(() => (window as unknown as { __registered: Descriptor[] }).__registered);
}

async function requests(page: Page): Promise<Recorded[]> {
  const body = (await (await page.request.get('/agent-fixture/requests')).json()) as { requests: Recorded[] };
  return body.requests;
}

/** Count the document loads of `page` from now on: an Inertia visit makes none. */
function documentLoads(page: Page): () => number {
  let loads = 0;
  page.on('load', () => {
    loads += 1;
  });
  return () => loads;
}

async function register(page: Page) {
  const stamp = `${Date.now()}${Math.floor(Math.random() * 1000)}`;
  await page.goto('/register');
  await page.locator('#name').fill(`Agent ${stamp}`);
  await page.locator('#email').fill(`agent-${stamp}@example.test`);
  await page.locator('#password').fill(password);
  await page.locator('#password_confirmation').fill(password);
  await page.getByRole('button', { name: 'Register', exact: true }).click();
  await expect(page).toHaveURL(/\/(verify-email|dashboard)$/);
}

const FIRST_PAGE_TOOLS = ['fixture.bump', 'fixture.help', 'fixture.peek', 'fixture.widget.ping'];

test(`agent_014_${kit}_page_registers_the_server_tools_the_viewer_may_call`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  // A guest is never offered the tool only a signed-in user may call.
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  await register(page);
  await page.goto('/agent-fixture/first');
  await expect
    .poll(() => registered(page))
    .toEqual(['fixture.bump', 'fixture.help', 'fixture.mine', 'fixture.peek', 'fixture.widget.ping']);
});

test(`agent_014_${kit}_server_tool_annotations_follow_the_markings`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const sent = await descriptors(page);
  const sentFor = (name: string) => sent.find((tool) => tool.name === name);
  expect(sentFor('fixture.peek')?.annotations).toEqual({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
  });
  expect(sentFor('fixture.bump')?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
  });
  expect(sentFor('fixture.bump')?.title).toBe('Bump');
  expect(sentFor('fixture.bump')?.description).toBe('Add to the counter the first page shows.');
  // What Chromium kept: the two hints it knows.
  const kept = await page.evaluate(async () => {
    const context = (document as unknown as {
      modelContext: { getTools(): Promise<{ name: string; annotations: Record<string, boolean> }[]> };
    }).modelContext;
    return (await context.getTools()).find((tool) => tool.name === 'fixture.peek')?.annotations;
  });
  expect(kept).toMatchObject({ readOnlyHint: true, untrustedContentHint: true });

  await page.getByTestId('to-second').click();
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  const reset = (await descriptors(page)).find((tool) => tool.name === 'fixture.reset');
  expect(reset?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
  });
});

test(`agent_014_${kit}_inertia_visit_replaces_the_page_tools`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const loads = documentLoads(page);
  await page.getByTestId('to-second').click();
  await expect(page.getByRole('heading', { name: 'Second' })).toBeVisible();
  // The route's tools leave, the group's shared tool stays registered.
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  expect(loads()).toBe(0);
  expect(agent.removed).toEqual(expect.arrayContaining(['fixture.bump', 'fixture.peek']));
  expect(agent.removed).not.toContain('fixture.help');
});

test(`agent_014_${kit}_history_back_restores_that_pages_tools`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  await page.getByTestId('to-second').click();
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'First' })).toBeVisible();
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
});

test(`agent_014_${kit}_server_tool_call_carries_session_csrf_and_header`, async ({ page }) => {
  const agent = await webmcp(page);
  await register(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toContain('fixture.mine');
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.mine', {});
  expect(response.output?.isError).toBe(false);
  // The route's own guard saw the session's user, through a browser call.
  expect(response.output?.structuredContent).toMatchObject({ browser: true });
  expect(response.output?.structuredContent?.user).toEqual(expect.any(String));
  const call = (await requests(page))
    .slice(before)
    .find((request) => request.path === '/agent-fixture/mine');
  expect(call).toMatchObject({ method: 'POST', tool: 'fixture.mine', csrf: true, cookie: true });
});

test(`agent_014_${kit}_write_tool_reloads_the_page_props`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const shown = Number(await page.getByTestId('counter').textContent());
  const loads = documentLoads(page);
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.bump', { by: 2 });
  expect(response.output?.isError).toBe(false);
  expect(response.output?.structuredContent).toMatchObject({ counter: shown + 2, kind: 'public' });
  // The screen shows what the agent did, through an Inertia reload.
  await expect(page.getByTestId('counter')).toHaveText(String(shown + 2));
  expect(loads()).toBe(0);
  const after = (await requests(page)).slice(before);
  const call = after.findIndex((request) => request.tool === 'fixture.bump');
  const reload = after.findIndex(
    (request) => request.method === 'GET' && request.path === '/agent-fixture/first' && request.inertia,
  );
  expect(call).toBeGreaterThanOrEqual(0);
  expect(reload).toBeGreaterThan(call);
  // A read-only tool reloads nothing.
  const quiet = (await requests(page)).length;
  expect((await agent.invoke('fixture.peek', {})).output?.isError).toBe(false);
  expect(
    (await requests(page)).slice(quiet).filter((request) => request.path === '/agent-fixture/first'),
  ).toEqual([]);
});

test(`agent_015_${kit}_component_tool_registers_and_leaves_with_its_component`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toContain('fixture.widget.ping');
  expect((await agent.invoke('fixture.widget.ping', {})).output?.structuredContent).toEqual({ pong: true });
  await page.getByTestId('toggle').click();
  await expect(page.getByTestId('widget')).toHaveCount(0);
  await expect.poll(() => registered(page)).not.toContain('fixture.widget.ping');
  await expect.poll(() => agent.removed).toContain('fixture.widget.ping');
  await page.getByTestId('toggle').click();
  await expect.poll(() => registered(page)).toContain('fixture.widget.ping');
});

test(`agent_015_${kit}_form_tool_registers_the_generated_schema`, async ({ page }) => {
  await page.goto('/agent-fixture/form');
  await expect
    .poll(() => registered(page))
    .toEqual(['fixture.help', 'fixture.notes.create', 'fixture.notes.create_and_leave', 'fixture.notes.purge']);
  const server = await (await page.request.get('/agent-fixture/schema')).json();
  const sent = (await descriptors(page)).find((tool) => tool.name === 'fixture.notes.create');
  // The schema `generate-types` wrote is the one the server derives.
  expect(sent?.inputSchema).toEqual(server);
  expect(sent?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
  });
});

test(`agent_015_${kit}_form_tool_fills_and_submits_through_inertia`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.create');
  const loads = documentLoads(page);
  const before = (await requests(page)).length;
  const title = `Agent note ${Date.now()}`;
  const response = await agent.invoke('fixture.notes.create', { title });
  expect(response.output?.isError).toBe(false);
  expect(response.output?.structuredContent).toMatchObject({ outcome: 'submitted', component: 'AgentFixture/Form' });
  await expect(page.getByTestId('notes')).toContainText(title);
  expect(loads()).toBe(0);
  // An ordinary Inertia visit: no tool-call header, the route's own checks.
  const submit = (await requests(page))
    .slice(before)
    .find((request) => request.method === 'POST' && request.path === '/agent-fixture/notes');
  expect(submit).toMatchObject({ inertia: true, tool: null, csrf: true });
});

test(`agent_015_${kit}_form_tool_returns_each_invalid_field`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.create');
  const response = await agent.invoke('fixture.notes.create', { title: 'ab' });
  expect(response.status).not.toBe('Error');
  expect(response.output?.isError).toBe(true);
  expect(response.output?.content?.[0]?.text).toContain('- title: Give the note a title of 3 to 80 characters.');
  expect(response.output?.structuredContent).toMatchObject({ outcome: 'invalid' });
  // The user sees the same form the agent filled, with the same error.
  await expect(page.getByTestId('title')).toHaveValue('ab');
  await expect(page.getByTestId('title-error')).toHaveText('Give the note a title of 3 to 80 characters.');
});

test(`agent_015_${kit}_destructive_form_tool_leaves_the_submit_to_the_user`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.purge');
  const sent = (await descriptors(page)).find((tool) => tool.name === 'fixture.notes.purge');
  expect(sent?.annotations.consequentialHint).toBe(true);
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.notes.purge', { confirm: 'yes' });
  expect(response.output?.structuredContent).toEqual({ outcome: 'filled', submitted: false });
  await expect(page.getByTestId('confirm')).toHaveValue('yes');
  const purges = async () =>
    (await requests(page))
      .slice(before)
      .filter((request) => request.method === 'POST' && request.path === '/agent-fixture/purge');
  expect(await purges()).toEqual([]);
  // Only the user's own click submits it.
  await page.getByTestId('purge').click();
  await expect.poll(async () => (await purges()).length).toBe(1);
  expect((await purges())[0]).toMatchObject({ inertia: true, tool: null });
});

test(`agent_015_${kit}_form_tool_answers_when_its_submit_leaves_the_page`, async ({ page }) => {
  const agent = await webmcp(page);
  // The submit redirects to the first page, so the form and its tool leave
  // the page while the agent's call still waits for its answer. Five times,
  // so a result the browser drops at the tool's removal shows.
  for (let round = 0; round < 5; round += 1) {
    await page.goto('/agent-fixture/form');
    await expect.poll(() => registered(page)).toContain('fixture.notes.create_and_leave');
    const title = `Leaving note ${round} ${Date.now()}`;
    const response = await agent.invoke('fixture.notes.create_and_leave', { title });
    expect(response.output?.isError, `round ${round}: ${JSON.stringify(response)}`).toBe(false);
    expect(response.output?.structuredContent).toMatchObject({
      outcome: 'submitted',
      component: 'AgentFixture/First',
    });
    await expect(page.getByRole('heading', { name: 'First' })).toBeVisible();
    await expect.poll(() => registered(page)).not.toContain('fixture.notes.create_and_leave');
  }
});
