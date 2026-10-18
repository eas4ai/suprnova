import { expect, test, type Page } from '@playwright/test';

// AGENT-012: Live tool actions registered with WebMCP, observed the way a
// built-in agent observes them: through the DevTools `WebMCP` domain and
// `document.modelContext`, with each call made by `WebMCP.invokeTool`.

interface ToolResponse {
  readonly status: string;
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
    readonly content?: { text: string }[];
  };
}

interface Recorded {
  readonly path: string;
  readonly tool: string | null;
  readonly status: number;
  readonly checks: Record<string, string | null>;
  readonly phases: string[];
  readonly gate: string[];
}

async function webmcp(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable');
  const added: string[] = [];
  const removed: string[] = [];
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded', (event: { tools: { name: string; frameId: string }[] }) => {
    for (const tool of event.tools) {
      added.push(tool.name);
      frameId = tool.frameId;
    }
  });
  cdp.on('WebMCP.toolsRemoved', (event: { tools: { name: string }[] }) => {
    for (const tool of event.tools) removed.push(tool.name);
  });
  cdp.on('WebMCP.toolResponded', (event: ToolResponse & { invocationId: string }) => {
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

async function registered(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools()).map((tool) => tool.name).sort();
  });
}

async function openTools(page: Page) {
  const agent = await webmcp(page);
  await page.goto('/tools');
  await expect
    .poll(() => registered(page))
    .toEqual(['counter.add.left', 'counter.add.main', 'counter.add.right']);
  return agent;
}

async function signIn(page: Page, user: string) {
  await page.goto(`/login/${user}`);
}

async function reset(page: Page) {
  await page.request.get('/test/reset');
}

async function actionRequests(page: Page): Promise<Recorded[]> {
  const body = (await (await page.request.get('/test/requests')).json()) as { requests: Recorded[] };
  return body.requests.filter((request) => request.path === '/__live/action');
}

function count(page: Page, key: string) {
  return page.locator(`[data-suprnova-live-document-key="${key}"] [data-testid="count"]`);
}

async function clickAdd(page: Page, key: string, expected: string) {
  await page.locator(`[data-suprnova-live-document-key="${key}"] [data-testid="add"]`).click();
  await expect(count(page, key)).toHaveText(expected);
}

/// Run the island's `touch` action, which anyone may run, so the public
/// seed becomes an instance and the next response is applied in place.
async function touch(page: Page, key: string) {
  const responded = page.waitForResponse('**/__live/action');
  await page.locator(`[data-suprnova-live-document-key="${key}"] [data-testid="touch"]`).click();
  expect((await responded).status()).toBe(200);
}

test('agent_012_mounted_tool_actions_register_with_model_context', async ({ page }) => {
  const agent = await openTools(page);
  await expect.poll(() => [...agent.added].sort()).toEqual([
    'counter.add.left',
    'counter.add.main',
    'counter.add.right',
  ]);
  const described = await page.evaluate(async () => {
    const context = (document as unknown as {
      modelContext: { getTools(): Promise<{ name: string; description: string; title: string; annotations: Record<string, boolean> }[]> };
    }).modelContext;
    return (await context.getTools()).find((tool) => tool.name === 'counter.add.main');
  });
  expect(described?.description).toBe('Add to the counter.');
  expect(described?.title).toBe('Add');
  expect(described?.annotations).toMatchObject({ readOnlyHint: false, untrustedContentHint: false });
});

test('agent_012_tool_names_carry_the_live_key', async ({ page }) => {
  await openTools(page);
  const keys = await page
    .locator('[data-suprnova-live-island]')
    .evaluateAll((roots) => roots.map((root) => root.getAttribute('data-suprnova-live-document-key')));
  expect((await registered(page)).map((name) => name.replace(/^counter\.add\./u, ''))).toEqual(
    [...keys].sort(),
  );
});

test('agent_012_tools_leave_with_their_component', async ({ page }) => {
  const agent = await openTools(page);
  await page.locator('#remove-first').click();
  await expect.poll(() => registered(page)).toEqual(['counter.add.left', 'counter.add.right']);
  await expect.poll(() => agent.removed).toContain('counter.add.main');
});

test('agent_012_tools_leave_on_navigation', async ({ page }) => {
  await openTools(page);
  await page.locator('#other').click();
  await page.waitForURL('**/other');
  await expect.poll(() => registered(page)).toEqual([]);
});

test('agent_012_tool_call_sends_the_island_snapshot_and_passes_the_csrf_and_session_checks', async ({
  page,
}) => {
  await signIn(page, 'alice');
  const agent = await openTools(page);
  // The first action promotes the public seed; the next click and the tool
  // call then act on the same instanced island.
  await clickAdd(page, 'main', '1');
  await reset(page);
  await clickAdd(page, 'main', '2');
  const response = await agent.invoke('counter.add.main', { amount: 3 });
  expect(response.status).toBe('Completed');
  expect(response.output?.isError).toBe(false);
  // The island applied the call's response as it applies a click's.
  await expect(count(page, 'main')).toHaveText('5');
  const [click, call] = await actionRequests(page);
  expect(click?.tool).toBeNull();
  expect(call?.tool).toBe('counter.add.main');
  expect(call?.status).toBe(200);
  // The same session, origin and CSRF evidence as the click's: the
  // browser's same-origin fetch is the Live origin proof the CSRF check
  // accepts.
  expect(call?.checks).toEqual(click?.checks);
  expect(call?.checks.session).toBe('Passed');
  expect(call?.checks.origin).toBe('Passed');
  expect(call?.checks.csrf).not.toBeNull();
});

test('agent_012_tool_call_is_authorized', async ({ page }) => {
  await signIn(page, 'bob');
  const agent = await openTools(page);
  await touch(page, 'main');
  await reset(page);
  await agent.invoke('counter.add.main', { amount: 5 });
  const [call] = await actionRequests(page);
  // The call reached the action's own policy, as bob, and stopped there:
  // the endpoint refused it before the action ran.
  expect(call?.tool).toBe('counter.add.main');
  expect(call?.gate).toEqual(['live:examples.agents-counter.add as bob']);
  expect(call?.phases).toContain('Authorize');
  expect(call?.phases).not.toContain('Action');
  expect(call?.status).toBe(409);
  // The island recovers as it does after a refused click, unchanged. That
  // recovery reloads the page, which can end the WebMCP invocation before
  // its error result reaches the agent.
  await expect(count(page, 'main')).toHaveText('0');
});

test('agent_012_tool_call_runs_the_execution_phases_of_a_click', async ({ page }) => {
  await signIn(page, 'alice');
  const agent = await openTools(page);
  await clickAdd(page, 'main', '1');
  await reset(page);
  await clickAdd(page, 'main', '2');
  const response = await agent.invoke('counter.add.main', { amount: 1 });
  expect(response.output?.isError).toBe(false);
  const [click, call] = await actionRequests(page);
  expect(click?.phases.length).toBeGreaterThan(0);
  expect(call?.phases).toEqual(click?.phases);
  for (const phase of ['Claim', 'Authorize', 'Validate', 'Action', 'Sign', 'LedgerAcceptance']) {
    expect(call?.phases).toContain(phase);
  }
});

test('agent_012_tool_result_carries_only_agent_visible_fields', async ({ page }) => {
  await signIn(page, 'alice');
  const agent = await openTools(page);
  const response = await agent.invoke('counter.add.main', { amount: 4 });
  expect(response.output?.isError).toBe(false);
  expect(response.output?.structuredContent).toEqual({
    outcome: 'accepted',
    fields: { count: '4' },
  });
  expect(JSON.stringify(response.output)).not.toContain('owner-only-the-server-sees');
});
